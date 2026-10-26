#[test]
fn chat_runtime_drives_provider_io_while_the_caller_is_synchronously_blocked() {
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::adapters::credentials::{CredentialSource, CredentialSourceError};
    use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelInputIR,
    };
    use crate::ports::model::{ModelProvider, ModelRequest, ModelStreamEvent};
    use futures_util::StreamExt;

    struct FixtureCredential;
    impl CredentialSource for FixtureCredential {
        fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
            Ok(Some("runtime-loopback-only".to_owned()))
        }
    }

    let runtime = crate::chat_runtime().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let (ready_sender, ready) = tokio::sync::oneshot::channel();
    let (release, released) = mpsc::channel();
    let (finished, finish) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = socket.read(&mut buffer).unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let header = std::str::from_utf8(&request[..end]).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        // The request is already in flight; the response is gated by the
        // caller entering its deliberately blocking persistence analogue.
        ready_sender.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(5)).unwrap();
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"network progressed\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n"
        );
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        socket.flush().unwrap();
    });
    let provider = ResponsesHttpAdapter::new(
        ResponsesHttpConfig::new(
            endpoint,
            "runtime-fixture",
            "CYBER_AGENT_RUNTIME_FIXTURE_API_KEY",
            ModelCapabilities {
                text_input: true,
                image_input: false,
                streaming: true,
                tool_call: true,
                structured_output: false,
                incremental_input: false,
                usage_reporting: false,
                max_output_tokens: None,
                reasoning_effort: None,
                context_window: 32_768,
                capability_mapping_version: CapabilityMappingVersion::new(1),
            },
        ),
        std::sync::Arc::new(FixtureCredential),
    );
    let mut stream = provider
        .stream(ModelRequest::full(ModelInputIR::current(
            "fixture",
            vec![ContentItem::user_text("hello")],
            Default::default(),
        )))
        .unwrap();
    let network = runtime.spawn(async move {
        while let Some(event) = stream.next().await {
            if matches!(event.unwrap(), ModelStreamEvent::Completed(_)) {
                finished.send(()).unwrap();
                return;
            }
        }
        panic!("fixture stream closed without completion");
    });
    runtime.block_on(async {
        ready.await.unwrap();
        release.send(()).unwrap();
        // There is no elapsed-time performance threshold: this handshake can
        // complete only if the runtime drives I/O independently of this caller.
        finish
            .recv_timeout(Duration::from_secs(5))
            .expect("Provider I/O must complete while the Chat caller is synchronously blocked");
        network.await.unwrap();
    });
    server.join().unwrap();
}
