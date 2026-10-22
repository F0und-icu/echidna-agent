#[allow(dead_code)]
#[path = "../src/adapters/subscription_auth/mod.rs"]
mod subscription_auth;
mod support;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use subscription_auth::{API_BASE_URL, AuthEndpoints, SubscriptionAuthService};
use support::TemporaryDirectory;

const TEST_KEY_ID: &str = "synthetic-test-key";
const TEST_MODULUS: &str = "ytNu3Ek5u84iuxLHVotlIugx4KbTo9xtN_AbeosoA3b8fe9_7EX1j8Hah6iLi7FI5_jn1cCEVLCepoXWV-R-P3MVigqdoAiClX2sVmLhyDZ57p8nK1iQSBTeuklOHXYMk5GKTL43P3-cJ8O5V-lSj4CYACrQxV4_B0tbwxAZOj3H98Jog52FhhO5xZ6AfNadTbIvIbWSnVZ06TmAUSie8T1jeaB-2ABVB_GtVshazXPo7T-rFJlWF2kOqDy4e7p5oNxD1SgbN_IUTL8iqUgnb74ySO_ZFcugI_gUywIb2_l38VlzLLZ1XXURumFrlTyjVxDC7zx_hJq0eLjiZgpvBQ";

#[derive(Clone)]
struct PresentedLogin {
    redirect_uri: String,
    state: String,
    nonce: String,
}

fn endpoints(base: &str) -> AuthEndpoints {
    AuthEndpoints {
        authorize_url: format!("{base}/authorize"),
        token_url: format!("{base}/token"),
        discovery_url: format!("{base}/discovery"),
        api_base_url: format!("{base}/v1"),
        issuer: format!("{base}/issuer"),
    }
}

fn service(store: &Path, base: &str) -> SubscriptionAuthService {
    service_as(store, base, "personal")
}

fn service_as(store: &Path, base: &str, account: &str) -> SubscriptionAuthService {
    SubscriptionAuthService::with_endpoints(
        store.to_path_buf(),
        account.to_owned(),
        endpoints(base),
    )
    .expect("synthetic endpoint service")
}

#[test]
fn both_project_runtime_directories_are_rejected_as_auth_store_paths() {
    let root = TemporaryDirectory::new("auth-project-directory-isolation");
    for name in [".echidna", ".cyber-agent"] {
        let store = root.path().join(name).join("auth");
        let result = SubscriptionAuthService::with_endpoints(
            store.clone(),
            "personal".to_owned(),
            endpoints("http://127.0.0.1:1"),
        );
        let error = match result {
            Ok(_) => panic!("project runtime directory must not become an auth store"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "subscription authentication store path must be an absolute private path"
        );
        assert!(!store.exists());
    }
}

#[tokio::test]
async fn signed_loopback_login_uses_pkce_and_models_preserve_server_order() {
    let directory = TemporaryDirectory::new("subscription-auth-success");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (presented_sender, presented_receiver) = mpsc::channel();
    let server_base = base.clone();
    let server = thread::spawn(move || {
        let presented = presented_receiver.recv().unwrap();
        send_callback(&presented, "issued-client", false);

        let (token_request, token_stream) = receive_request(&listener);
        assert!(token_request.starts_with("POST /token HTTP/1.1"));
        assert!(token_request.contains("grant_type=authorization_code"));
        assert!(token_request.contains("client_id=issued-client"));
        assert!(token_request.contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"));
        respond(
            token_stream,
            &format!(
                "{{\"access_token\":\"synthetic-access\",\"refresh_token\":\"synthetic-refresh\",\"expires_in\":3600,\"scope\":\"chatgpt.tokens.use.direct\",\"id_token\":\"{}\"}}",
                signed_identity(&server_base, "issued-client", &presented.nonce)
            ),
        );

        let (discovery_request, discovery_stream) = receive_request(&listener);
        assert!(discovery_request.starts_with("GET /discovery HTTP/1.1"));
        respond(
            discovery_stream,
            &format!("{{\"issuer\":\"{server_base}/issuer\",\"jwks_uri\":\"{server_base}/jwks\"}}"),
        );
        let (jwks_request, jwks_stream) = receive_request(&listener);
        assert!(jwks_request.starts_with("GET /jwks HTTP/1.1"));
        respond(jwks_stream, &jwks());

        let (model_request, model_stream) = receive_request(&listener);
        assert!(model_request.starts_with("GET /v1/models HTTP/1.1"));
        assert!(
            model_request
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-access")
        );
        respond(
            model_stream,
            "{\"models\":[{\"slug\":\"hidden\",\"display_name\":\"Hidden\",\"visibility\":\"hidden\"},{\"slug\":\"first\",\"display_name\":\"First\",\"visibility\":\"list\",\"default_reasoning_level\":\"high\",\"supported_reasoning_levels\":[{\"effort\":\"high\",\"description\":\"Ignored by SIWC\"}]},{\"slug\":\"second\",\"display_name\":\"Second\",\"visibility\":\"list\"}]}",
        );
    });

    let auth = service(&store, &base);
    let status = auth
        .login(|url| {
            assert!(url.starts_with(&format!("{base}/authorize?")));
            assert!(url.contains("client_id=dynamic_agent_client"));
            assert!(url.contains(&format!(
                "resource={}",
                API_BASE_URL.replace(':', "%3A").replace('/', "%2F")
            )));
            assert!(url.contains("scope=openid%20profile%20email%20offline_access%20resource.invoke%20chatgpt.tokens.use.direct"));
            assert!(url.contains("code_challenge_method=S256"));
            let values = query(url);
            let state = values.get("state").unwrap();
            let nonce = values.get("nonce").unwrap();
            assert_eq!(state.len(), 43);
            assert_eq!(nonce.len(), 43);
            assert!(values.get("code_challenge").is_some_and(|value| value.len() == 43));
            presented_sender
                .send(PresentedLogin {
                    redirect_uri: values.get("redirect_uri").unwrap().to_owned(),
                    state: state.to_owned(),
                    nonce: nonce.to_owned(),
                })
                .unwrap();
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&status).unwrap()["state"], "connected");
    assert!(status.verified_identity);
    assert!(status.direct_access_granted);
    let models = auth.models().await.unwrap();
    assert_eq!(
        serde_json::to_value(&models).unwrap(),
        serde_json::json!([
            {"slug": "first", "display_name": "First"},
            {"slug": "second", "display_name": "Second"}
        ])
    );
    assert_eq!(
        models
            .into_iter()
            .map(|model| model.slug)
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    server.join().unwrap();

    let connection = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(connection.contains("issued-client"));
    assert!(connection.contains("synthetic-access"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&store).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            std::fs::metadata(store.join("personal.connection"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

#[tokio::test]
async fn siwc_models_reject_missing_visibility_instead_of_silently_filtering_it() {
    let directory = TemporaryDirectory::new("subscription-models-missing-visibility");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let expires_at = now_unix() + 3600;
    seed_connection(&store, expires_at, expires_at);
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("GET /v1/models HTTP/1.1"));
        respond(
            stream,
            r#"{"models":[{"slug":"missing","display_name":"Missing"}]}"#,
        );
    });

    assert_eq!(
        service(&store, &base)
            .models()
            .await
            .unwrap_err()
            .to_string(),
        "subscription response is invalid"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn callback_state_and_initial_registration_fail_before_token_exchange() {
    for (label, callback, expected) in [
        (
            "bad-state",
            "state=wrong&error=access_denied",
            "subscription login callback is invalid",
        ),
        (
            "missing-client",
            "code=synthetic-code",
            "subscription client registration is invalid",
        ),
        (
            "denied",
            "error=access_denied",
            "subscription login was denied",
        ),
    ] {
        let directory = TemporaryDirectory::new(label);
        let store = canonical_temp_path(directory.path().join("auth-store"));
        let auth = service(&store, "http://127.0.0.1:1");
        let callback_streams = Arc::new(Mutex::new(Vec::new()));
        let retained_streams = Arc::clone(&callback_streams);
        let result = auth
            .login(|url| {
                let values = query(url);
                let mut query = callback.to_owned();
                if !query.contains("state=") {
                    query.push_str("&state=");
                    query.push_str(values.get("state").unwrap());
                }
                let redirect = values.get("redirect_uri").unwrap();
                let mut stream = TcpStream::connect(callback_address(redirect)).unwrap();
                write!(stream, "GET /auth/callback?{query} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
                retained_streams.lock().unwrap().push(stream);
                Ok(())
            })
            .await;
        drop(callback_streams);
        assert_eq!(result.unwrap_err().to_string(), expected);
        assert!(!store.join("personal.connection").exists());
    }
}

#[tokio::test]
async fn signed_identity_without_direct_scope_is_rejected_without_persisting_a_connection() {
    let directory = TemporaryDirectory::new("subscription-missing-direct-scope");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (presented_sender, presented_receiver) = mpsc::channel();
    let server_base = base.clone();
    let server = thread::spawn(move || {
        let presented = presented_receiver.recv().unwrap();
        send_callback(&presented, "issued-client", false);
        let (token, stream) = receive_request(&listener);
        assert!(token.starts_with("POST /token HTTP/1.1"));
        respond(
            stream,
            &format!(
                "{{\"access_token\":\"synthetic-access\",\"refresh_token\":\"synthetic-refresh\",\"expires_in\":3600,\"scope\":\"openid\",\"id_token\":\"{}\"}}",
                signed_identity(&server_base, "issued-client", &presented.nonce)
            ),
        );
        let (_, stream) = receive_request(&listener);
        respond(
            stream,
            &format!("{{\"issuer\":\"{server_base}/issuer\",\"jwks_uri\":\"{server_base}/jwks\"}}"),
        );
        let (_, stream) = receive_request(&listener);
        respond(stream, &jwks());
    });
    let auth = service(&store, &base);
    let error = auth
        .login(|url| {
            let values = query(url);
            presented_sender
                .send(PresentedLogin {
                    redirect_uri: values.get("redirect_uri").unwrap().to_owned(),
                    state: values.get("state").unwrap().to_owned(),
                    nonce: values.get("nonce").unwrap().to_owned(),
                })
                .unwrap();
            Ok(())
        })
        .await
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error.to_string(),
        "subscription direct token permission was not granted"
    );
    assert!(!store.join("personal.connection").exists());
}

#[tokio::test]
async fn login_http_failures_identify_the_safe_operation_and_status_without_persisting() {
    #[derive(Clone, Copy)]
    enum FailureStage {
        TokenExchange,
        Discovery,
        Jwks,
    }

    for (label, stage, status, operation) in [
        (
            "token-400",
            FailureStage::TokenExchange,
            400,
            "token exchange",
        ),
        (
            "discovery-401",
            FailureStage::Discovery,
            401,
            "OpenID discovery",
        ),
        ("jwks-403", FailureStage::Jwks, 403, "JWKS retrieval"),
        (
            "token-429",
            FailureStage::TokenExchange,
            429,
            "token exchange",
        ),
    ] {
        let directory = TemporaryDirectory::new(label);
        let store = canonical_temp_path(directory.path().join("auth-store"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (presented_sender, presented_receiver) = mpsc::channel();
        let server_base = base.clone();
        let server = thread::spawn(move || {
            let presented = presented_receiver.recv().unwrap();
            send_callback(&presented, "issued-client", false);

            let (token_request, token_stream) = receive_request(&listener);
            assert!(token_request.starts_with("POST /token HTTP/1.1"));
            if matches!(stage, FailureStage::TokenExchange) {
                respond_with_status(token_stream, status, "oauth-stage-secret");
                return;
            }
            respond(
                token_stream,
                "{\"access_token\":\"synthetic-access\",\"refresh_token\":\"synthetic-refresh\",\"expires_in\":3600,\"scope\":\"chatgpt.tokens.use.direct\",\"id_token\":\"synthetic-id-token\"}",
            );

            let (discovery_request, discovery_stream) = receive_request(&listener);
            assert!(discovery_request.starts_with("GET /discovery HTTP/1.1"));
            if matches!(stage, FailureStage::Discovery) {
                respond_with_status(discovery_stream, status, "oauth-stage-secret");
                return;
            }
            respond(
                discovery_stream,
                &format!(
                    "{{\"issuer\":\"{server_base}/issuer\",\"jwks_uri\":\"{server_base}/jwks\"}}"
                ),
            );

            let (jwks_request, jwks_stream) = receive_request(&listener);
            assert!(jwks_request.starts_with("GET /jwks HTTP/1.1"));
            respond_with_status(jwks_stream, status, "oauth-stage-secret");
        });

        let auth = service(&store, &base);
        let error = auth
            .login(|url| {
                let values = query(url);
                presented_sender
                    .send(PresentedLogin {
                        redirect_uri: values.get("redirect_uri").unwrap().to_owned(),
                        state: values.get("state").unwrap().to_owned(),
                        nonce: values.get("nonce").unwrap().to_owned(),
                    })
                    .unwrap();
                Ok(())
            })
            .await
            .unwrap_err();
        server.join().unwrap();

        let message = error.to_string();
        assert_eq!(
            message,
            format!("subscription authentication {operation} request failed with HTTP {status}")
        );
        assert!(!message.contains("oauth-stage-secret"));
        assert!(!format!("{error:?}").contains("oauth-stage-secret"));
        assert!(!store.join("personal.connection").exists());
    }
}

#[tokio::test]
async fn reconnect_callback_must_match_the_saved_client_but_may_omit_it() {
    let directory = TemporaryDirectory::new("subscription-reconnect-client");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    seed_connection(&store, now_unix() + 3600, now_unix() + 3500);
    let auth = service(&store, "http://127.0.0.1:1");
    let mismatch = callback_login(&auth, "code=synthetic-code&client_id=other-client").await;
    assert_eq!(
        mismatch.unwrap_err().to_string(),
        "subscription client registration is invalid"
    );

    let absent = callback_login(&auth, "code=synthetic-code").await;
    assert_eq!(
        absent.unwrap_err().to_string(),
        "subscription authentication service is temporarily unavailable"
    );
    assert!(
        std::fs::read_to_string(store.join("personal.connection"))
            .unwrap()
            .contains("issued-client")
    );
}

#[tokio::test]
async fn reconnect_subject_mismatch_is_rejected_after_signed_validation() {
    let directory = TemporaryDirectory::new("subscription-reconnect-subject");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    seed_connection_with_identity(
        &store,
        "personal",
        "issued-client",
        "synthetic-subject",
        &format!("{base}/issuer"),
    );
    let (presented_sender, presented_receiver) = mpsc::channel();
    let server_base = base.clone();
    let server = thread::spawn(move || {
        let presented = presented_receiver.recv().unwrap();
        send_callback_without_client(&presented);
        let (_, stream) = receive_request(&listener);
        respond(
            stream,
            &format!(
                "{{\"access_token\":\"synthetic-access\",\"refresh_token\":\"synthetic-refresh\",\"expires_in\":3600,\"scope\":\"chatgpt.tokens.use.direct\",\"id_token\":\"{}\"}}",
                signed_identity_for_subject(
                    &server_base,
                    "issued-client",
                    &presented.nonce,
                    "other-subject"
                )
            ),
        );
        let (_, stream) = receive_request(&listener);
        respond(
            stream,
            &format!("{{\"issuer\":\"{server_base}/issuer\",\"jwks_uri\":\"{server_base}/jwks\"}}"),
        );
        let (_, stream) = receive_request(&listener);
        respond(stream, &jwks());
    });
    let auth = service(&store, &base);
    let error = auth
        .login(|url| {
            let values = query(url);
            presented_sender
                .send(PresentedLogin {
                    redirect_uri: values.get("redirect_uri").unwrap().to_owned(),
                    state: values.get("state").unwrap().to_owned(),
                    nonce: values.get("nonce").unwrap().to_owned(),
                })
                .unwrap();
            Ok(())
        })
        .await
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error.to_string(),
        "subscription identity does not match the selected account"
    );
    assert!(
        std::fs::read_to_string(store.join("personal.connection"))
            .unwrap()
            .contains("synthetic-subject")
    );
}

#[tokio::test]
async fn account_labels_share_only_the_root_host_identity() {
    let directory = TemporaryDirectory::new("subscription-host-identity");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let personal = service_as(&store, "http://127.0.0.1:1", "personal");
    let error = callback_login(&personal, "code=synthetic-code")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "subscription client registration is invalid"
    );
    let first_host = std::fs::read_to_string(store.join("host")).unwrap();
    assert!(first_host.starts_with("{\"version\":1,\"host_id\":\"urn:uuid:"));

    let work = service_as(&store, "http://127.0.0.1:1", "work");
    let error = callback_login(&work, "code=synthetic-code")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "subscription client registration is invalid"
    );
    assert_eq!(
        std::fs::read(store.join("host")).unwrap(),
        first_host.as_bytes()
    );

    let expires_at = now_unix() + 3600;
    let earliest_refresh_at = now_unix() + 3500;
    seed_connection_with_identity(
        &store,
        "personal",
        "personal-client",
        "personal-subject",
        "synthetic-issuer",
    );
    set_seeded_tokens(
        &store,
        "personal",
        "personal-access",
        "personal-refresh",
        expires_at,
        earliest_refresh_at,
    );
    seed_connection_with_identity(
        &store,
        "work",
        "work-client",
        "work-subject",
        "synthetic-issuer",
    );
    set_seeded_tokens(
        &store,
        "work",
        "work-access",
        "work-refresh",
        expires_at,
        earliest_refresh_at,
    );

    assert_eq!(personal.status().await.unwrap().account, "personal");
    assert_eq!(personal.access_token().await.unwrap(), "personal-access");
    assert_eq!(work.status().await.unwrap().account, "work");
    assert_eq!(work.access_token().await.unwrap(), "work-access");

    std::fs::copy(
        store.join("personal.connection"),
        store.join("work.connection"),
    )
    .unwrap();
    assert_eq!(
        work.status().await.unwrap_err().to_string(),
        "subscription authentication storage is malformed"
    );
}

async fn callback_login(auth: &SubscriptionAuthService, initial_query: &str) -> Result<(), String> {
    let callback_streams = Arc::new(Mutex::new(Vec::new()));
    let retained_streams = Arc::clone(&callback_streams);
    let result = auth
        .login(|url| {
            let values = query(url);
            let mut query = initial_query.to_owned();
            query.push_str("&state=");
            query.push_str(values.get("state").unwrap());
            let mut stream = TcpStream::connect(callback_address(values.get("redirect_uri").unwrap())).unwrap();
            write!(stream, "GET /auth/callback?{query} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
            retained_streams.lock().unwrap().push(stream);
            Ok(())
        })
        .await;
    drop(callback_streams);
    result.map(|_| ()).map_err(|error| error.to_string())
}

#[tokio::test]
async fn status_is_read_only_and_refuses_an_unsafe_existing_store() {
    let directory = TemporaryDirectory::new("subscription-status");
    let store = canonical_temp_path(directory.path().join("never-created"));
    let auth = service(&store, "http://127.0.0.1:1");
    let status = auth.status().await.unwrap();
    assert_eq!(
        serde_json::to_value(&status).unwrap()["state"],
        "disconnected"
    );
    assert!(!store.exists());

    #[cfg(unix)]
    {
        let target = directory.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &store).unwrap();
        let error = auth.status().await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "subscription authentication storage contains a symbolic link"
        );
        assert!(!error.to_string().contains("synthetic-access"));

        let private_parent = directory.path().join("private-parent");
        std::fs::create_dir(&private_parent).unwrap();
        let linked_parent = directory.path().join("linked-parent");
        std::os::unix::fs::symlink(&private_parent, &linked_parent).unwrap();
        let through_parent_link = service(&linked_parent.join("auth-store"), "http://127.0.0.1:1");
        assert_eq!(
            through_parent_link.status().await.unwrap_err().to_string(),
            "subscription authentication storage contains a symbolic link"
        );
    }
}

#[tokio::test]
async fn refresh_rotates_atomically_and_temporary_or_terminal_failures_preserve_the_right_state() {
    let directory = TemporaryDirectory::new("subscription-refresh");

    let rotated_store = canonical_temp_path(directory.path().join("rotated"));
    seed_connection(&rotated_store, now_unix() + 120, now_unix() - 1);
    let (base, server) = one_response_server(
        200,
        "{\"access_token\":\"synthetic-next-access\",\"refresh_token\":\"synthetic-next-refresh\",\"expires_in\":3600,\"scope\":\"chatgpt.tokens.use.direct\"}",
    );
    let auth = service(&rotated_store, &base);
    assert_eq!(auth.access_token().await.unwrap(), "synthetic-next-access");
    server.join().unwrap();
    let rotated = std::fs::read_to_string(rotated_store.join("personal.connection")).unwrap();
    assert!(rotated.contains("synthetic-next-access"));
    assert!(rotated.contains("synthetic-next-refresh"));
    assert!(!rotated.contains("synthetic-access"));

    let temporary_store = canonical_temp_path(directory.path().join("temporary"));
    seed_connection(&temporary_store, now_unix() + 120, now_unix() - 1);
    let (base, server) = one_response_server(503, "temporary synthetic failure");
    let auth = service(&temporary_store, &base);
    assert_eq!(auth.access_token().await.unwrap(), "synthetic-access");
    server.join().unwrap();
    assert!(
        std::fs::read_to_string(temporary_store.join("personal.connection"))
            .unwrap()
            .contains("synthetic-refresh")
    );

    let terminal_store = canonical_temp_path(directory.path().join("terminal"));
    seed_connection(&terminal_store, now_unix() - 1, now_unix() - 2);
    let (base, server) = one_response_server(401, "{\"error\":\"invalid_grant\"}");
    let auth = service(&terminal_store, &base);
    let error = auth.access_token().await.unwrap_err();
    server.join().unwrap();
    assert_eq!(error.to_string(), "subscription credentials require login");
    let cleared = std::fs::read_to_string(terminal_store.join("personal.connection")).unwrap();
    assert!(cleared.contains("\"tokens\":null"));
    assert!(!cleared.contains("synthetic-access"));

    let unknown_store = canonical_temp_path(directory.path().join("unknown"));
    seed_connection(&unknown_store, now_unix() + 120, now_unix() - 1);
    let (base, server) = one_response_server(400, "{\"error\":\"unclassified_failure\"}");
    let auth = service(&unknown_store, &base);
    assert_eq!(auth.access_token().await.unwrap(), "synthetic-access");
    server.join().unwrap();
    assert!(
        std::fs::read_to_string(unknown_store.join("personal.connection"))
            .unwrap()
            .contains("synthetic-refresh")
    );

    let malformed_store = canonical_temp_path(directory.path().join("malformed"));
    seed_connection(&malformed_store, now_unix() + 120, now_unix() - 1);
    let (base, server) = one_response_server(200, "not json");
    let auth = service(&malformed_store, &base);
    assert_eq!(auth.access_token().await.unwrap(), "synthetic-access");
    server.join().unwrap();
    assert!(
        std::fs::read_to_string(malformed_store.join("personal.connection"))
            .unwrap()
            .contains("synthetic-refresh")
    );

    let invalid_client_store = canonical_temp_path(directory.path().join("invalid-client"));
    seed_connection(&invalid_client_store, now_unix() + 120, now_unix() - 1);
    let (base, server) = one_response_server(400, "{\"error\":\"invalid_client\"}");
    let auth = service(&invalid_client_store, &base);
    assert_eq!(
        auth.access_token().await.unwrap_err().to_string(),
        "subscription client registration is invalid"
    );
    server.join().unwrap();
    assert!(
        std::fs::read_to_string(invalid_client_store.join("personal.connection"))
            .unwrap()
            .contains("synthetic-refresh")
    );

    let delayed_store = canonical_temp_path(directory.path().join("delayed"));
    seed_connection(&delayed_store, now_unix() + 1, now_unix() - 1);
    let (base, server) = delayed_response_server(503, "temporary synthetic failure");
    let auth = service(&delayed_store, &base);
    assert_eq!(
        auth.access_token().await.unwrap_err().to_string(),
        "subscription authentication service is temporarily unavailable"
    );
    server.join().unwrap();
    assert!(
        std::fs::read_to_string(delayed_store.join("personal.connection"))
            .unwrap()
            .contains("synthetic-refresh")
    );

    let not_before_store = canonical_temp_path(directory.path().join("not-before"));
    seed_connection(&not_before_store, now_unix() - 1, now_unix() + 300);
    let auth = service(&not_before_store, "http://127.0.0.1:1");
    assert_eq!(
        auth.access_token().await.unwrap_err().to_string(),
        "subscription credentials cannot yet be refreshed"
    );
}

#[tokio::test]
async fn refresh_redirect_is_not_followed_or_forwarded() {
    let directory = TemporaryDirectory::new("subscription-refresh-redirect");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    seed_connection(&store, now_unix() + 120, now_unix() - 1);
    let (base, server) = redirect_response_server();
    let auth = service(&store, &base);

    assert_eq!(auth.access_token().await.unwrap(), "synthetic-access");
    server.join().unwrap();
}

#[tokio::test]
async fn stored_tokens_with_a_swapped_registration_binding_fail_closed() {
    let directory = TemporaryDirectory::new("subscription-swapped-binding");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    seed_connection(&store, now_unix() + 3600, now_unix() + 3500);
    let path = store.join("personal.connection");
    let swapped = std::fs::read_to_string(&path).unwrap().replacen(
        "\"tokens\":{\"version\":1,\"client_id\":\"issued-client\"",
        "\"tokens\":{\"version\":1,\"client_id\":\"swapped-client\"",
        1,
    );
    std::fs::write(&path, swapped).unwrap();
    let error = service(&store, "http://127.0.0.1:1")
        .status()
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "subscription authentication storage is malformed"
    );
    assert!(!error.to_string().contains("synthetic-access"));
}

#[tokio::test]
async fn logout_clears_local_tokens_when_remote_revocation_cannot_be_confirmed() {
    let directory = TemporaryDirectory::new("subscription-logout");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    seed_connection(&store, now_unix() + 3600, now_unix() + 3500);
    let auth = service(&store, "http://127.0.0.1:1");
    assert_eq!(
        serde_json::to_value(auth.logout().await.unwrap()).unwrap(),
        "remote_unconfirmed_cleared"
    );
    let connection = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(connection.contains("\"tokens\":null"));
    assert!(!connection.contains("synthetic-refresh"));
}

#[tokio::test]
async fn logout_reports_confirmed_revocation_only_after_the_fake_service_confirms_it() {
    let directory = TemporaryDirectory::new("subscription-confirmed-logout");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    seed_connection(&store, now_unix() + 3600, now_unix() + 3500);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server_base = base.clone();
    let server = thread::spawn(move || {
        let (discovery, stream) = receive_request(&listener);
        assert!(discovery.starts_with("GET /discovery HTTP/1.1"));
        respond(
            stream,
            &format!(
                "{{\"issuer\":\"{server_base}/issuer\",\"jwks_uri\":\"{server_base}/jwks\",\"revocation_endpoint\":\"{server_base}/revoke\"}}"
            ),
        );
        let (revoke, stream) = receive_request(&listener);
        assert!(revoke.starts_with("POST /revoke HTTP/1.1"));
        assert!(revoke.contains("token=synthetic-refresh"));
        respond(stream, "{}");
    });
    let auth = service(&store, &base);
    assert_eq!(
        serde_json::to_value(auth.logout().await.unwrap()).unwrap(),
        "revoked_and_cleared"
    );
    server.join().unwrap();
    assert!(
        std::fs::read_to_string(store.join("personal.connection"))
            .unwrap()
            .contains("\"tokens\":null")
    );
}

#[test]
fn two_processes_serialize_one_refresh_and_reread_the_rotated_connection() {
    let directory = TemporaryDirectory::new("subscription-cross-process-refresh");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    seed_connection(&store, now_unix() + 120, now_unix() - 1);
    let (base, server) = one_response_server(
        200,
        "{\"access_token\":\"synthetic-next-access\",\"refresh_token\":\"synthetic-next-refresh\",\"expires_in\":3600,\"scope\":\"chatgpt.tokens.use.direct\"}",
    );
    let worker = std::env::current_exe().unwrap();
    let first = thread::spawn({
        let worker = worker.clone();
        let store = store.clone();
        let base = base.clone();
        move || run_refresh_worker(worker, store, base)
    });
    let second = thread::spawn({
        let store = store.clone();
        move || run_refresh_worker(worker, store, base)
    });
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    server.join().unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let connection = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(connection.contains("synthetic-next-access"));
    assert!(!connection.contains("synthetic-access"));
}

#[tokio::test]
async fn refresh_subprocess_worker() {
    let Ok(store) = std::env::var("ECHIDNA_SUBSCRIPTION_AUTH_TEST_STORE") else {
        return;
    };
    let endpoint = std::env::var("ECHIDNA_SUBSCRIPTION_AUTH_TEST_ENDPOINT").unwrap();
    assert_eq!(
        service(Path::new(&store), &endpoint)
            .access_token()
            .await
            .unwrap(),
        "synthetic-next-access"
    );
}

fn run_refresh_worker(
    worker: std::path::PathBuf,
    store: std::path::PathBuf,
    endpoint: String,
) -> std::process::Output {
    Command::new(worker)
        .args([
            "--exact",
            "refresh_subprocess_worker",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ECHIDNA_SUBSCRIPTION_AUTH_TEST_STORE", store)
        .env("ECHIDNA_SUBSCRIPTION_AUTH_TEST_ENDPOINT", endpoint)
        .output()
        .unwrap()
}

fn seed_connection(store: &Path, expires_at_unix: u64, earliest_refresh_at_unix: u64) {
    seed_connection_with_identity(
        store,
        "personal",
        "issued-client",
        "synthetic-subject",
        "synthetic-issuer",
    );
    set_seeded_tokens(
        store,
        "personal",
        "synthetic-access",
        "synthetic-refresh",
        expires_at_unix,
        earliest_refresh_at_unix,
    );
}

fn set_seeded_tokens(
    store: &Path,
    account: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at_unix: u64,
    earliest_refresh_at_unix: u64,
) {
    let path = store.join(format!("{account}.connection"));
    let contents = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "\"access_token\":\"synthetic-access\"",
            &format!("\"access_token\":\"{access_token}\""),
        )
        .replace(
            "\"refresh_token\":\"synthetic-refresh\"",
            &format!("\"refresh_token\":\"{refresh_token}\""),
        )
        .replace(
            "\"expires_at_unix\":0",
            &format!("\"expires_at_unix\":{expires_at_unix}"),
        )
        .replace(
            "\"earliest_refresh_at_unix\":0",
            &format!("\"earliest_refresh_at_unix\":{earliest_refresh_at_unix}"),
        );
    std::fs::write(path, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(store.join(format!("{account}.connection")))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

fn seed_connection_with_identity(
    store: &Path,
    account: &str,
    client_id: &str,
    subject: &str,
    issuer: &str,
) {
    std::fs::create_dir_all(store).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(store, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let connection = format!(
        "{{\"version\":1,\"registration\":{{\"version\":1,\"account\":\"{account}\",\"client_id\":\"{client_id}\",\"subject\":\"{subject}\",\"issuer\":\"{issuer}\"}},\"tokens\":{{\"version\":1,\"client_id\":\"{client_id}\",\"subject\":\"{subject}\",\"issuer\":\"{issuer}\",\"access_token\":\"synthetic-access\",\"refresh_token\":\"synthetic-refresh\",\"expires_at_unix\":0,\"earliest_refresh_at_unix\":0,\"scopes\":[\"chatgpt.tokens.use.direct\"]}}}}"
    );
    let path = store.join(format!("{account}.connection"));
    std::fs::write(&path, connection).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn one_response_server(status: u16, body: &'static str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("POST /token HTTP/1.1"));
        assert!(request.contains("grant_type=refresh_token"));
        assert!(request.contains("refresh_token=synthetic-refresh"));
        respond_with_status(stream, status, body);
    });
    (base, server)
}

fn delayed_response_server(status: u16, body: &'static str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("POST /token HTTP/1.1"));
        assert!(request.contains("grant_type=refresh_token"));
        assert!(request.contains("refresh_token=synthetic-refresh"));
        thread::sleep(std::time::Duration::from_secs(2));
        respond_with_status(stream, status, body);
    });
    (base, server)
}

fn redirect_response_server() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let redirect_target = format!("{base}/redirected");
    let server = thread::spawn(move || {
        let (request, mut stream) = receive_request(&listener);
        assert!(request.starts_with("POST /token HTTP/1.1"));
        assert!(request.contains("grant_type=refresh_token"));
        assert!(request.contains("refresh_token=synthetic-refresh"));
        write!(
            stream,
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect_target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        stream.flush().unwrap();
        assert_no_followup_request(&listener);
    });
    (base, server)
}

fn assert_no_followup_request(listener: &TcpListener) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        match listener.accept() {
            Ok((_stream, _)) => panic!("OAuth client followed redirect with a second request"),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return;
                }
                thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("fake OAuth service accept failed: {error}"),
        }
    }
}

fn respond_with_status(mut stream: TcpStream, status: u16, body: &str) {
    write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn receive_request(listener: &TcpListener) -> (String, TcpStream) {
    listener.set_nonblocking(true).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "fake OAuth service did not receive the expected request"
                );
                thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("fake OAuth service accept failed: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length: ")
                .or_else(|| line.strip_prefix("Content-Length: "))
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() < header_end + length {
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    (String::from_utf8(bytes).unwrap(), stream)
}

fn respond(mut stream: TcpStream, body: &str) {
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

fn send_callback(presented: &PresentedLogin, client_id: &str, denied: bool) {
    let address = callback_address(&presented.redirect_uri);
    let query = if denied {
        format!("state={}&error=access_denied", presented.state)
    } else {
        format!(
            "state={}&code=synthetic-code&client_id={client_id}",
            presented.state
        )
    };
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET /auth/callback?{query} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("Echidna authorization callback received."));
    assert!(!response.contains("Echidna sign-in complete."));
}

fn send_callback_without_client(presented: &PresentedLogin) {
    let address = callback_address(&presented.redirect_uri);
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET /auth/callback?state={}&code=synthetic-code HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        presented.state
    )
    .unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut response = [0_u8; 128];
    assert!(stream.read(&mut response).unwrap() > 0);
}

fn callback_address(uri: &str) -> String {
    uri.strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_owned()
}

fn query(url: &str) -> BTreeMap<String, String> {
    url.split_once('?')
        .unwrap()
        .1
        .split('&')
        .map(|pair| pair.split_once('=').unwrap())
        .map(|(key, value)| (key.to_owned(), percent_decode(value)))
        .collect()
}

fn percent_decode(value: &str) -> String {
    let mut output = Vec::new();
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().unwrap();
            let low = bytes.next().unwrap();
            output.push((hex(high) << 4) | hex(low));
        } else {
            output.push(byte);
        }
    }
    String::from_utf8(output).unwrap()
}

fn hex(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'A'..=b'F' => byte - b'A' + 10,
        _ => byte - b'a' + 10,
    }
}

fn signed_identity(base: &str, client_id: &str, nonce: &str) -> String {
    signed_identity_for_subject(base, client_id, nonce, "synthetic-subject")
}

fn signed_identity_for_subject(base: &str, client_id: &str, nonce: &str, subject: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let header = encode("{\"alg\":\"RS256\",\"kid\":\"synthetic-test-key\"}");
    let claims = encode(&format!(
        "{{\"iss\":\"{base}/issuer\",\"aud\":\"{client_id}\",\"exp\":{},\"iat\":{},\"nonce\":\"{nonce}\",\"sub\":\"{subject}\"}}",
        now + 3600,
        now
    ));
    let signed = format!("{header}.{claims}");
    let key = RsaKeyPair::from_pkcs8(include_bytes!(
        "fixtures/subscription_auth/test-rsa-2048.pk8"
    ))
    .unwrap();
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        signed.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!(
        "{signed}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    )
}

fn jwks() -> String {
    format!(
        "{{\"keys\":[{{\"kid\":\"{TEST_KEY_ID}\",\"kty\":\"RSA\",\"n\":\"{TEST_MODULUS}\",\"e\":\"AQAB\",\"alg\":\"RS256\",\"use\":\"sig\"}}]}}"
    )
}

fn encode(value: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value)
}

fn canonical_temp_path(path: std::path::PathBuf) -> std::path::PathBuf {
    path.parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(path.file_name().unwrap())
}
