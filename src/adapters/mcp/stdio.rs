//! One Unix stdio session: framed RPC and synchronous process-group cleanup.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard};
use tokio::task::JoinHandle;

use super::observation::{McpObservation, McpObserver};
use super::rpc::{MAX_MESSAGE_BYTES, decode_message};
use super::{CompletedCall, McpError, McpServerConfig};
use crate::domain::canonical_json::canonical_blake3_hash;

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const BASE_ENV: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "LC_CTYPE", "TZ",
];

pub(super) struct StdioClient {
    identity: String,
    connection: AsyncMutex<Option<Connection>>,
    process: Mutex<Option<OwnedProcess>>,
    timeout: Duration,
    startup_timeout: Duration,
    next_id: AtomicU64,
}

struct Connection {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

// No child stderr text is retained. The reader owns a fixed buffer only.
#[derive(Default)]
struct StderrMetadata {
    bytes: AtomicU64,
    error: Mutex<Option<io::ErrorKind>>,
}

struct OwnedProcess {
    server_id: String,
    observer: Option<McpObserver>,
    child: Child,
    stderr: Option<JoinHandle<()>>,
    metadata: Arc<StderrMetadata>,
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if let Some(reader) = self.stderr.take() {
            reader.abort();
        }
        // Never reap/try_wait before this point: the unreaped group leader
        // keeps its PID/PGID reserved until the group signal has been sent.
        let group = self.child.id() as libc::pid_t;
        // SAFETY: this is our unreaped child's independent process group.
        let group_error = if unsafe { libc::killpg(group, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            (error.raw_os_error() != Some(libc::ESRCH)).then_some(error.kind())
        } else {
            None
        };
        let kill_error = self.child.kill().err().map(|error| error.kind());
        let wait_error = self.child.wait().err().map(|error| error.kind());
        if (group_error.is_some() || wait_error.is_some())
            && let Some(observer) = &self.observer
        {
            observer(McpObservation::Cleanup {
                server_id: self.server_id.clone(),
                group_error,
                kill_error,
                wait_error,
                stderr_bytes: self.metadata.bytes.load(Ordering::Relaxed),
                stderr_error: *self
                    .metadata
                    .error
                    .lock()
                    .expect("MCP stderr mutex poisoned"),
            });
        }
    }
}

impl StdioClient {
    pub(super) fn start(
        server_id: &str,
        config: &McpServerConfig,
        observer: Option<McpObserver>,
    ) -> Result<Self, McpError> {
        let command = config.command.as_ref().expect("validated stdio command");
        let cwd = config.cwd.as_ref().expect("resolved stdio cwd");
        let args = config.args.as_deref().unwrap_or_default();
        let mut environment = BTreeMap::<String, OsString>::new();
        for key in BASE_ENV {
            if let Some(value) = std::env::var_os(key) {
                environment.insert((*key).to_owned(), value);
            }
        }
        for (key, value) in config.env.iter().flatten() {
            environment.insert(key.clone(), value.into());
        }
        let identity = canonical_blake3_hash(&(
            command.as_os_str().as_encoded_bytes(),
            args,
            cwd.as_os_str().as_encoded_bytes(),
            environment
                .iter()
                .map(|(key, value)| (key, value.as_encoded_bytes()))
                .collect::<Vec<_>>(),
        ))
        .map_err(|_| protocol("stdio launch identity could not be fingerprinted"))?;
        let child = Command::new(command)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(environment)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| transport("stdio spawn", error.kind()))?;
        let metadata = Arc::new(StderrMetadata::default());
        let mut owned = OwnedProcess {
            server_id: server_id.to_owned(),
            observer,
            child,
            stderr: None,
            metadata: Arc::clone(&metadata),
        };
        let input = ChildStdin::from_std(owned.child.stdin.take().expect("piped stdio stdin"))
            .map_err(|error| transport("stdio stdin setup", error.kind()))?;
        let output = ChildStdout::from_std(owned.child.stdout.take().expect("piped stdio stdout"))
            .map_err(|error| transport("stdio stdout setup", error.kind()))?;
        let mut stderr = tokio::process::ChildStderr::from_std(
            owned.child.stderr.take().expect("piped stdio stderr"),
        )
        .map_err(|error| transport("stdio stderr setup", error.kind()))?;
        owned.stderr = Some(tokio::spawn(async move {
            let mut buffer = [0_u8; 8192];
            loop {
                match stderr.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(count) => {
                        let _ = metadata.bytes.fetch_update(
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                            |old| Some(old.saturating_add(count as u64)),
                        );
                    }
                    Err(error) => {
                        *metadata.error.lock().expect("MCP stderr mutex poisoned") =
                            Some(error.kind());
                        break;
                    }
                }
            }
        }));
        Ok(Self {
            identity: format!("stdio:{identity}"),
            connection: AsyncMutex::new(Some(Connection {
                input,
                output: BufReader::new(output),
            })),
            process: Mutex::new(Some(owned)),
            timeout: Duration::from_millis(config.timeout_ms),
            startup_timeout: Duration::from_millis(config.startup_timeout_ms()),
            next_id: AtomicU64::new(1),
        })
    }

    pub(super) fn identity(&self) -> &str {
        &self.identity
    }

    pub(super) fn shutdown(&self) {
        let process = self
            .process
            .lock()
            .expect("MCP process mutex poisoned")
            .take();
        if let Ok(mut connection) = self.connection.try_lock() {
            connection.take();
        }
        drop(process);
    }

    pub(super) async fn initialize(&self) -> Result<(), McpError> {
        let result = match self.request("initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"echidna","version":env!("CARGO_PKG_VERSION")}}), true).await? {
            CompletedCall::Result(result) => result,
            CompletedCall::Error(_) => return Err(protocol("stdio initialize returned a JSON-RPC error")),
        };
        if result["protocolVersion"] != "2025-11-25"
            || !result
                .get("capabilities")
                .and_then(|c| c.get("tools"))
                .is_some_and(Value::is_object)
        {
            self.shutdown();
            return Err(protocol(
                "stdio initialize did not confirm protocol and tools",
            ));
        }
        self.request("notifications/initialized", json!({}), false)
            .await?;
        Ok(())
    }

    pub(super) async fn list_tools(&self, cursor: Option<&str>) -> Result<Value, McpError> {
        let params = cursor.map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
        match self.request("tools/list", params, true).await? {
            CompletedCall::Result(result) => Ok(result),
            CompletedCall::Error(_) => Err(protocol("stdio tools/list returned a JSON-RPC error")),
        }
    }

    pub(super) async fn call(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<CompletedCall, McpError> {
        self.request(
            "tools/call",
            json!({"name":name,"arguments":arguments}),
            true,
        )
        .await
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        response: bool,
    ) -> Result<CompletedCall, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let mut body = json!({"jsonrpc":"2.0","method":method,"params":params});
        if response {
            body["id"] = json!(id);
        }
        let mut attempted = false;
        let operation = async {
            let connection = self.connection.lock().await;
            if connection.is_none()
                || self
                    .process
                    .lock()
                    .expect("MCP process mutex poisoned")
                    .is_none()
            {
                return Err(McpError::PreExecution {
                    message: "stdio MCP session is closed; no request sent".to_owned(),
                });
            }
            let mut request = RequestGuard {
                client: self,
                connection,
                armed: false,
            };
            let mut material = 0;
            let bytes = encode_frame(&body, &mut material).map_err(|_| McpError::PreExecution {
                message: "stdio request could not be encoded within 2 MiB; no request sent"
                    .to_owned(),
            })?;
            request.armed = true;
            attempted = true;
            let io = request.connection.as_mut().expect("validated connection");
            io.input
                .write_all(&bytes)
                .await
                .map_err(|e| transport("stdio write", e.kind()))?;
            io.input
                .flush()
                .await
                .map_err(|e| transport("stdio flush", e.kind()))?;
            if !response {
                request.armed = false;
                return Ok(CompletedCall::Result(json!({})));
            }
            loop {
                let value = io.read_frame(&mut material).await?;
                if value.get("method").is_none() {
                    let result = decode_message(value, &id)?;
                    request.armed = false;
                    return Ok(result);
                }
                if value["jsonrpc"] != "2.0"
                    || !value["method"].is_string()
                    || value.get("result").is_some()
                    || value.get("error").is_some()
                    || value
                        .get("params")
                        .is_some_and(|params| !params.is_object())
                {
                    return Err(protocol("invalid stdio server message"));
                }
                match value.get("id") {
                    None => match value["method"].as_str() {
                        Some("notifications/message" | "notifications/progress") => {}
                        _ => return Err(protocol("unsupported stdio notification")),
                    },
                    Some(server_id)
                        if server_id.is_string() || server_id.is_i64() || server_id.is_u64() =>
                    {
                        let ping = value["method"] == "ping";
                        let reply = if ping {
                            json!({"jsonrpc":"2.0","id":server_id,"result":{}})
                        } else {
                            json!({"jsonrpc":"2.0","id":server_id,"error":{"code":-32601,"message":"Method not found"}})
                        };
                        let bytes = encode_frame(&reply, &mut material)?;
                        io.input
                            .write_all(&bytes)
                            .await
                            .map_err(|e| transport("stdio server reply", e.kind()))?;
                        if !ping {
                            return Err(protocol("unsupported stdio server request"));
                        }
                    }
                    _ => return Err(protocol("invalid stdio server request id")),
                }
            }
        };
        match tokio::time::timeout(
            if method == "tools/call" {
                self.timeout
            } else {
                self.startup_timeout
            },
            operation,
        )
        .await
        {
            Ok(result) => result,
            Err(_) if attempted => Err(protocol("stdio request timed out after dispatch")),
            Err(_) => Err(McpError::PreExecution {
                message: "stdio request timed out before dispatch".to_owned(),
            }),
        }
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct RequestGuard<'a> {
    client: &'a StdioClient,
    connection: MutexGuard<'a, Option<Connection>>,
    armed: bool,
}

impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.connection.take();
            self.client.shutdown();
        }
    }
}

impl Connection {
    async fn read_frame(&mut self, material: &mut usize) -> Result<Value, McpError> {
        let mut frame = Vec::new();
        loop {
            let available = self
                .output
                .fill_buf()
                .await
                .map_err(|e| transport("stdio read", e.kind()))?;
            if available.is_empty() {
                return Err(protocol(
                    "stdio EOF before a final newline JSON-RPC response",
                ));
            }
            let count = available
                .iter()
                .position(|b| *b == b'\n')
                .map_or(available.len(), |p| p + 1);
            if frame.len().saturating_add(count) > MAX_MESSAGE_BYTES {
                return Err(protocol("stdio frame exceeds 2 MiB"));
            }
            count_material(material, count)?;
            frame.extend_from_slice(&available[..count]);
            self.output.consume(count);
            if frame.last() == Some(&b'\n') {
                return serde_json::from_slice(&frame)
                    .map_err(|_| protocol("stdio frame was not valid JSON"));
            }
        }
    }
}

fn encode_frame(value: &Value, material: &mut usize) -> Result<Vec<u8>, McpError> {
    let mut bytes =
        serde_json::to_vec(value).map_err(|_| protocol("stdio request could not be encoded"))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(protocol("stdio frame exceeds 2 MiB"));
    }
    count_material(material, bytes.len())?;
    Ok(bytes)
}

fn count_material(material: &mut usize, bytes: usize) -> Result<(), McpError> {
    *material = material.saturating_add(bytes);
    if *material > MAX_REQUEST_BYTES {
        return Err(protocol("stdio request material exceeds 8 MiB"));
    }
    Ok(())
}
fn protocol(message: &str) -> McpError {
    McpError::Protocol {
        message: message.to_owned(),
    }
}
fn transport(stage: &str, kind: io::ErrorKind) -> McpError {
    McpError::Transport {
        message: format!("{stage}: {kind:?}"),
    }
}
