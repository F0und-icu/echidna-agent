#[path = "../src/adapters/http_error_body.rs"]
pub(crate) mod http_error_body;
#[allow(dead_code)]
#[path = "../src/model_catalog.rs"]
mod model_catalog;
mod adapters {
    pub(crate) use crate::http_error_body;
}
#[allow(dead_code)]
#[path = "../src/adapters/subscription_auth/mod.rs"]
mod subscription_auth;
mod support;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::{Value, json};
use subscription_auth::{AuthEndpoints, CodexAuthEndpoints, SubscriptionAuthService};
use support::TemporaryDirectory;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const TEST_KEY_ID: &str = "synthetic-test-key";
const TEST_MODULUS: &str = "ytNu3Ek5u84iuxLHVotlIugx4KbTo9xtN_AbeosoA3b8fe9_7EX1j8Hah6iLi7FI5_jn1cCEVLCepoXWV-R-P3MVigqdoAiClX2sVmLhyDZ57p8nK1iQSBTeuklOHXYMk5GKTL43P3-cJ8O5V-lSj4CYACrQxV4_B0tbwxAZOj3H98Jog52FhhO5xZ6AfNadTbIvIbWSnVZ06TmAUSie8T1jeaB-2ABVB_GtVshazXPo7T-rFJlWF2kOqDy4e7p5oNxD1SgbN_IUTL8iqUgnb74ySO_ZFcugI_gUywIb2_l38VlzLLZ1XXURumFrlTyjVxDC7zx_hJq0eLjiZgpvBQ";

fn endpoints(base: &str) -> CodexAuthEndpoints {
    CodexAuthEndpoints {
        authorize_url: format!("{base}/authorize"),
        token_url: format!("{base}/token"),
        revoke_url: format!("{base}/revoke"),
        discovery_url: format!("{base}/discovery"),
        api_base_url: format!("{base}/codex"),
        issuer: format!("{base}/issuer"),
    }
}

fn service(store: &Path, base: &str) -> SubscriptionAuthService {
    service_with_catalog_client_version(store, base, None)
}

fn service_with_catalog_client_version(
    store: &Path,
    base: &str,
    catalog_client_version: Option<&str>,
) -> SubscriptionAuthService {
    SubscriptionAuthService::with_codex_endpoints(
        store.to_path_buf(),
        "personal".to_owned(),
        endpoints(base),
        catalog_client_version,
    )
    .unwrap()
}

#[tokio::test]
async fn codex_login_uses_fixed_oauth_requests_and_account_bound_model_discovery() {
    let directory = TemporaryDirectory::new("codex-subscription-auth");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (callback_sender, callback_receiver) = mpsc::channel::<(String, String)>();
    let server_base = base.clone();
    let server = thread::spawn(move || {
        let (redirect_uri, state) = callback_receiver.recv().unwrap();
        send_callback(&redirect_uri, &state);
        let (token_request, token_stream) = receive_request(&listener);
        assert!(token_request.starts_with("POST /token HTTP/1.1"));
        assert!(token_request.contains("grant_type=authorization_code"));
        assert!(token_request.contains(&format!("client_id={CLIENT_ID}")));
        assert!(token_request.contains("code=synthetic-code"));
        assert!(token_request.contains("code_verifier="));
        assert!(!token_request.contains("resource="));
        assert!(!token_request.contains("nonce="));
        respond(
            token_stream,
            &json!({
                "id_token": signed_identity(&server_base, "subject-a", "workspace-a"),
                "access_token": display_access_token(now_unix() + 3600, "login@example.test", "Login Name", "plus"),
                "refresh_token": "synthetic-refresh"
            })
            .to_string(),
        );

        let (discovery_request, discovery_stream) = receive_request(&listener);
        assert!(discovery_request.starts_with("GET /discovery HTTP/1.1"));
        respond(
            discovery_stream,
            &json!({"issuer": format!("{server_base}/issuer"), "jwks_uri": format!("{server_base}/jwks")}).to_string(),
        );
        let (jwks_request, jwks_stream) = receive_request(&listener);
        assert!(jwks_request.starts_with("GET /jwks HTTP/1.1"));
        respond(jwks_stream, &jwks());

        let (models_request, models_stream) = receive_request(&listener);
        assert!(models_request.starts_with(&format!(
            "GET /codex/models?client_version={} HTTP/1.1",
            subscription_auth::CODEX_CATALOG_COMPATIBILITY_VERSION
        )));
        assert!(
            models_request
                .to_ascii_lowercase()
                .contains("authorization: bearer e30.")
        );
        assert!(
            models_request
                .to_ascii_lowercase()
                .contains("chatgpt-account-id: workspace-a")
        );
        assert!(
            models_request
                .to_ascii_lowercase()
                .contains("version: 0.144.0")
        );
        assert!(
            models_request
                .to_ascii_lowercase()
                .contains("user-agent: echidna/0.0.1")
        );
        assert!(!models_request.to_ascii_lowercase().contains("originator:"));
        respond(
            models_stream,
            r#"{"models":[{"slug":"second","display_name":"Second","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"medium","description":"Balanced"},{"effort":"high","description":"Thorough"}],"context_window":128000,"max_context_window":256000,"input_modalities":["text","image","future"],"supports_reasoning_summary_parameter":true},{"slug":"first","display_name":"First","supported_reasoning_levels":[{"effort":"future","description":"Future server option"}]},{"slug":"null-window","display_name":"Null window","context_window":null,"max_context_window":null}]}"#,
        );
    });

    // Reserve the first fixed port so this regression proves the bounded
    // 1455-to-1457 fallback before the authorization URL is exposed.
    let _first_port = TcpListener::bind("127.0.0.1:1455").unwrap();
    let auth = service(&store, &base);
    let status = auth
        .login_codex(|url| {
            assert!(url.starts_with(&format!("{base}/authorize?")));
            let values = query(url);
            assert_eq!(
                values.get("response_type").map(String::as_str),
                Some("code")
            );
            assert_eq!(values.get("client_id").map(String::as_str), Some(CLIENT_ID));
            assert_eq!(
                values.get("redirect_uri").map(String::as_str),
                Some("http://localhost:1457/auth/callback")
            );
            assert_eq!(
                values.get("scope").map(String::as_str),
                Some("openid offline_access")
            );
            assert_eq!(
                values.get("id_token_add_organizations").map(String::as_str),
                Some("true")
            );
            assert_eq!(
                values.get("codex_cli_simplified_flow").map(String::as_str),
                Some("true")
            );
            assert_eq!(
                values.get("originator").map(String::as_str),
                Some("codex_cli_rs")
            );
            assert_eq!(
                values.get("code_challenge_method").map(String::as_str),
                Some("S256")
            );
            assert_eq!(values.get("state").map(String::len), Some(43));
            assert_eq!(values.get("code_challenge").map(String::len), Some(43));
            assert!(!values.contains_key("nonce"));
            assert!(!values.contains_key("resource"));
            assert!(!url.contains("chatgpt.tokens.use.direct"));
            callback_sender
                .send((
                    values.get("redirect_uri").unwrap().to_owned(),
                    values.get("state").unwrap().to_owned(),
                ))
                .unwrap();
            Ok(())
        })
        .await
        .unwrap();
    let status = serde_json::to_value(status).unwrap();
    assert_eq!(status["account"], "personal");
    assert_eq!(status["state"], "connected");
    assert_eq!(status["verified_identity"], true);
    assert_eq!(status["account_header_ready"], true);
    assert!(status["access_token_expires_at_unix"].as_u64() > Some(now_unix()));
    assert_eq!(status["email"], "login@example.test");
    assert_eq!(status["name"], "Login Name");
    assert_eq!(status["plan"], "plus");
    assert!(!status.to_string().contains("synthetic-refresh"));
    assert!(!status.to_string().contains("signature"));
    assert!(status.get("direct_access_granted").is_none());
    let models = auth.catalog().models().await.unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.slug.as_str())
            .collect::<Vec<_>>(),
        ["second", "first", "null-window"]
    );
    assert_eq!(models[0].default_reasoning_level.as_deref(), Some("high"));
    assert_eq!(models[0].context_window, Some(128_000));
    assert_eq!(models[0].max_context_window, Some(256_000));
    assert_eq!(
        models[0].input_modalities.as_deref(),
        Some(["text".to_owned(), "image".to_owned(), "future".to_owned()].as_slice())
    );
    assert_eq!(models[0].supports_reasoning_summary_parameter, Some(true));
    assert_eq!(
        models[0]
            .supported_reasoning_levels
            .iter()
            .map(|level| (level.effort.as_str(), level.description.as_str()))
            .collect::<Vec<_>>(),
        [("medium", "Balanced"), ("high", "Thorough")]
    );
    assert_eq!(models[1].default_reasoning_level, None);
    assert_eq!(models[1].supported_reasoning_levels[0].effort, "future");
    assert_eq!(models[1].context_window, None);
    assert_eq!(models[1].max_context_window, None);
    assert_eq!(models[1].input_modalities, None);
    assert_eq!(models[2].context_window, None);
    assert_eq!(models[2].max_context_window, None);
    server.join().unwrap();

    let stored = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(stored.contains("\"protocol\":\"codex\""));
    assert!(stored.contains("workspace-a"));
}

#[tokio::test]
async fn codex_and_siwc_stores_are_dispatched_without_fallback_or_rewrite() {
    let directory = TemporaryDirectory::new("codex-subscription-store-isolation");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    std::fs::create_dir(&store).unwrap();
    set_private_directory(&store);
    let connection = store.join("personal.connection");
    let legacy = r#"{"version":1,"registration":{"version":1,"account":"personal","client_id":"issued-client","subject":"subject","issuer":"issuer","future_field":true},"tokens":null,"future_field":true}"#;
    std::fs::write(&connection, legacy).unwrap();
    set_private_file(&connection);

    let codex = service(&store, "http://127.0.0.1:1");
    assert_eq!(
        codex
            .login_codex(|_| unreachable!("foreign store must fail before browser presentation"))
            .await
            .unwrap_err()
            .to_string(),
        "subscription authentication storage belongs to another protocol"
    );
    assert_eq!(std::fs::read_to_string(&connection).unwrap(), legacy);

    let siwc = SubscriptionAuthService::new(store.clone(), "personal".to_owned()).unwrap();
    assert!(
        siwc.status().await.is_ok(),
        "legacy SIWC bytes must stay readable"
    );

    let malformed_codex = r#"{"protocol":"codex","version":1,"registration":{"version":1,"account":"personal","client_id":"issued-client","subject":"subject","issuer":"issuer"},"tokens":null}"#;
    std::fs::write(&connection, malformed_codex).unwrap();
    set_private_file(&connection);
    assert_eq!(
        codex.codex_status().await.unwrap_err().to_string(),
        "subscription authentication storage is malformed"
    );
    assert_eq!(
        siwc.status().await.unwrap_err().to_string(),
        "subscription authentication storage is malformed"
    );
    assert_eq!(
        std::fs::read_to_string(&connection).unwrap(),
        malformed_codex
    );
}

#[tokio::test]
async fn codex_models_reject_ambiguous_or_malformed_catalog_metadata() {
    let cases = [
        (
            "duplicate-slug",
            r#"{"models":[{"slug":"same","display_name":"One"},{"slug":"same","display_name":"Two"}]}"#,
        ),
        (
            "duplicate-effort",
            r#"{"models":[{"slug":"one","display_name":"One","supported_reasoning_levels":[{"effort":"high","description":"First"},{"effort":"high","description":"Second"}]}]}"#,
        ),
        (
            "missing-description",
            r#"{"models":[{"slug":"one","display_name":"One","supported_reasoning_levels":[{"effort":"high"}]}]}"#,
        ),
        (
            "zero-context-window",
            r#"{"models":[{"slug":"one","display_name":"One","context_window":0}]}"#,
        ),
        (
            "negative-max-context-window",
            r#"{"models":[{"slug":"one","display_name":"One","max_context_window":-1}]}"#,
        ),
        (
            "fractional-context-window",
            r#"{"models":[{"slug":"one","display_name":"One","context_window":1.5}]}"#,
        ),
        (
            "overflow-max-context-window",
            r#"{"models":[{"slug":"one","display_name":"One","max_context_window":4294967296}]}"#,
        ),
        (
            "string-context-window",
            r#"{"models":[{"slug":"one","display_name":"One","context_window":"128000"}]}"#,
        ),
        (
            "null-input-modalities",
            r#"{"models":[{"slug":"one","display_name":"One","input_modalities":null}]}"#,
        ),
        (
            "null-summary-support",
            r#"{"models":[{"slug":"one","display_name":"One","supports_reasoning_summary_parameter":null}]}"#,
        ),
    ];
    for (label, body) in cases {
        let directory = TemporaryDirectory::new(&format!("codex-models-{label}"));
        let store = canonical_temp_path(directory.path().join("auth-store"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        seed_codex_connection(
            &store,
            &base,
            access_token(now_unix() + 3600),
            now_unix() + 3600,
        );
        let server = thread::spawn(move || {
            let (request, stream) = receive_request(&listener);
            assert!(request.starts_with("GET /codex/models?client_version=0.144.0 HTTP/1.1"));
            respond(stream, body);
        });

        assert_eq!(
            service(&store, &base)
                .catalog()
                .models()
                .await
                .unwrap_err()
                .0
                .to_string(),
            "subscription response is invalid"
        );
        server.join().unwrap();
    }
}

#[tokio::test]
async fn codex_catalog_client_version_controls_both_catalog_request_locations() {
    let directory = TemporaryDirectory::new("codex-catalog-client-version");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    seed_codex_connection(
        &store,
        &base,
        access_token(now_unix() + 3600),
        now_unix() + 3600,
    );
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("GET /codex/models?client_version=1.2.3 HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("version: 1.2.3"));
        respond(
            stream,
            r#"{"models":[{"slug":"gpt-new-server-id","display_name":"New server model"}]}"#,
        );
    });

    let models = service_with_catalog_client_version(&store, &base, Some("1.2.3"))
        .catalog()
        .models()
        .await
        .unwrap();
    assert_eq!(models[0].slug, "gpt-new-server-id");
    server.join().unwrap();
}

#[test]
fn codex_catalog_client_version_rejects_noncanonical_values_before_store_use() {
    let directory = TemporaryDirectory::new("codex-catalog-client-version-invalid");
    let store = canonical_temp_path(directory.path().join("uncreated-auth-store"));
    let invalid_versions = [
        "",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.3-rc1",
        "1.2.3+build",
        "1.2.3 ",
        "1.2.x",
        "4294967296.2.3",
    ];

    for version in invalid_versions {
        let error = match SubscriptionAuthService::with_codex_endpoints(
            store.clone(),
            "personal".to_owned(),
            endpoints("http://127.0.0.1:1"),
            Some(version),
        ) {
            Err(error) => error,
            Ok(_) => panic!("invalid catalog compatibility version was accepted"),
        };
        assert_eq!(
            error.to_string(),
            "subscription catalog compatibility version is invalid"
        );
        if !version.is_empty() {
            assert!(!error.to_string().contains(version));
            assert!(!format!("{error:?}").contains(version));
        }
        assert!(!store.exists(), "invalid version must not touch the store");
    }
}

#[tokio::test]
async fn codex_model_catalog_http_failures_keep_credentials_and_redact_remote_data() {
    for status in [400, 401, 403, 429, 500] {
        let directory = TemporaryDirectory::new("codex-model-catalog-http-failure");
        let store = canonical_temp_path(directory.path().join("auth-store"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        seed_codex_connection(
            &store,
            &base,
            access_token(now_unix() + 3600),
            now_unix() + 3600,
        );
        let before = std::fs::read_to_string(store.join("personal.connection")).unwrap();
        let server = thread::spawn(move || {
            let (request, stream) = receive_request(&listener);
            assert!(request.starts_with("GET /codex/models?client_version=0.144.0 HTTP/1.1"));
            respond_with_status_and_header(
                stream,
                status,
                r#"{"error":{"message":"catalog-response-secret"}}"#,
                Some("X-Catalog-Response-Secret: catalog-header-secret"),
            );
        });

        let (error, detail) = service(&store, &base).catalog().models().await.unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "subscription authentication model catalog request failed with HTTP {status}: {{\"error\":{{\"message\":\"catalog-response-secret\"}}}}"
            )
        );
        let debug = format!("{error:?}");
        assert!(error.to_string().contains("catalog-response-secret"));
        assert!(!error.to_string().contains("catalog-header-secret"));
        assert!(debug.contains("catalog-response-secret"));
        assert!(detail.is_none());
        assert!(!debug.contains("catalog-header-secret"));
        server.join().unwrap();
        assert_eq!(
            std::fs::read_to_string(store.join("personal.connection")).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn codex_refresh_uses_json_and_retains_omitted_identity_and_refresh_tokens() {
    let directory = TemporaryDirectory::new("codex-subscription-refresh-rotation");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let rotated_access = access_token(now_unix() + 3600);
    let (base, server) = token_server(
        200,
        json!({"access_token": rotated_access.clone()}).to_string(),
    );
    seed_codex_connection(&store, &base, access_token(now_unix() - 1), now_unix() - 1);

    service(&store, &base)
        .access_codex_credential()
        .await
        .unwrap();
    server.join().unwrap();

    let stored = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(stored.contains(&rotated_access));
    assert!(stored.contains("synthetic-refresh"));
    assert!(stored.contains("workspace-a"));
}

#[tokio::test]
async fn explicit_codex_refresh_rotates_a_future_token_and_returns_sanitized_status() {
    let directory = TemporaryDirectory::new("codex-explicit-refresh-rotation");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let rotated_access = display_access_token(
        now_unix() + 3600,
        "refresh@example.test",
        "Refresh Name",
        "pro",
    );
    let server_rotated_access = rotated_access.clone();
    let previous_access = access_token(now_unix() + 600);
    let server_base = base.clone();
    seed_codex_connection(&store, &base, previous_access.clone(), now_unix() + 600);
    let before =
        serde_json::to_value(service(&store, &base).codex_status().await.unwrap()).unwrap();
    assert_eq!(before["email"], Value::Null);
    assert_eq!(before["name"], Value::Null);
    assert_eq!(before["plan"], Value::Null);
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert_refresh_request(&request);
        respond(
            stream,
            &json!({
                "access_token": server_rotated_access,
                "refresh_token": "synthetic-next-refresh",
                "id_token": signed_identity(&server_base, "subject-a", "workspace-a")
            })
            .to_string(),
        );
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("GET /discovery HTTP/1.1"));
        respond(
            stream,
            &json!({"issuer": format!("{server_base}/issuer"), "jwks_uri": format!("{server_base}/jwks")}).to_string(),
        );
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("GET /jwks HTTP/1.1"));
        respond(stream, &jwks());
        assert_no_followup_request(&listener);
    });

    let status =
        serde_json::to_value(service(&store, &base).refresh_codex().await.unwrap()).unwrap();
    assert_eq!(status["account"], "personal");
    assert_eq!(status["state"], "connected");
    assert_eq!(status["verified_identity"], true);
    assert_eq!(status["account_header_ready"], true);
    assert!(status["access_token_expires_at_unix"].as_u64() > Some(now_unix()));
    assert_eq!(status["email"], "refresh@example.test");
    assert_eq!(status["name"], "Refresh Name");
    assert_eq!(status["plan"], "pro");
    assert!(!status.to_string().contains(&rotated_access));
    assert!(!status.to_string().contains("synthetic-next-refresh"));
    assert!(status.get("access_token").is_none());
    assert!(status.get("refresh_token").is_none());
    server.join().unwrap();

    let stored = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(stored.contains(&rotated_access));
    assert!(!stored.contains(&previous_access));
    assert!(stored.contains("synthetic-next-refresh"));
    assert!(!stored.contains("synthetic-refresh"));
}

#[tokio::test]
async fn explicit_codex_refresh_rejects_returned_identity_mismatch_without_writing() {
    let directory = TemporaryDirectory::new("codex-explicit-refresh-mismatch");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    seed_codex_connection(
        &store,
        &base,
        access_token(now_unix() + 3600),
        now_unix() + 3600,
    );
    let before = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    let server_base = base.clone();
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert_refresh_request(&request);
        respond(
            stream,
            &json!({
                "access_token": access_token(now_unix() + 3600),
                "id_token": signed_identity(&server_base, "subject-b", "workspace-a")
            })
            .to_string(),
        );
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("GET /discovery HTTP/1.1"));
        respond(
            stream,
            &json!({"issuer": format!("{server_base}/issuer"), "jwks_uri": format!("{server_base}/jwks")}).to_string(),
        );
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("GET /jwks HTTP/1.1"));
        respond(stream, &jwks());
    });

    assert_eq!(
        service(&store, &base)
            .refresh_codex()
            .await
            .unwrap_err()
            .to_string(),
        "subscription identity does not match the selected account"
    );
    server.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(store.join("personal.connection")).unwrap(),
        before
    );
}

#[tokio::test]
async fn explicit_codex_refresh_keeps_future_tokens_on_temporary_failure() {
    let directory = TemporaryDirectory::new("codex-explicit-refresh-temporary");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let (base, server) = token_server(500, r#"{"error":"temporary"}"#.to_owned());
    seed_codex_connection(
        &store,
        &base,
        access_token(now_unix() + 3600),
        now_unix() + 3600,
    );
    let before = std::fs::read_to_string(store.join("personal.connection")).unwrap();

    assert_eq!(
        service(&store, &base)
            .refresh_codex()
            .await
            .unwrap_err()
            .to_string(),
        "subscription authentication service is temporarily unavailable: HTTP 500: {\"error\":\"temporary\"}"
    );
    server.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(store.join("personal.connection")).unwrap(),
        before
    );
}

#[tokio::test]
async fn explicit_codex_refresh_terminal_failure_clears_only_tokens() {
    let directory = TemporaryDirectory::new("codex-explicit-refresh-terminal");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let (base, server) = token_server(400, r#"{"error":"invalid_grant"}"#.to_owned());
    seed_codex_connection(
        &store,
        &base,
        access_token(now_unix() + 3600),
        now_unix() + 3600,
    );

    assert_eq!(
        service(&store, &base)
            .refresh_codex()
            .await
            .unwrap_err()
            .to_string(),
        "subscription credentials require login: HTTP 400: {\"error\":\"invalid_grant\"}"
    );
    server.join().unwrap();
    let stored = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(stored.contains("workspace-a"));
    assert!(stored.contains("\"tokens\":null"));
}

#[tokio::test]
async fn unexpired_codex_credential_access_skips_network_refresh() {
    let directory = TemporaryDirectory::new("codex-unexpired-credential");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let access = access_token(now_unix() + 3600);
    seed_codex_connection(&store, &base, access.clone(), now_unix() + 3600);

    let credential = service(&store, &base)
        .access_codex_credential()
        .await
        .unwrap();
    assert_eq!(credential.into_parts(), (access, "workspace-a".to_owned()));
    assert_no_followup_request(&listener);
}

#[tokio::test]
async fn explicit_refresh_rejects_siwc_without_a_request() {
    let directory = TemporaryDirectory::new("siwc-explicit-refresh-rejected");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let siwc = SubscriptionAuthService::with_endpoints(
        store,
        "personal".to_owned(),
        AuthEndpoints {
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            discovery_url: format!("{base}/discovery"),
            api_base_url: format!("{base}/api"),
            issuer: format!("{base}/issuer"),
        },
    )
    .unwrap();

    assert_eq!(
        siwc.refresh_codex().await.unwrap_err().to_string(),
        "explicit credential refresh requires a Codex subscription Provider"
    );
    assert_no_followup_request(&listener);
}

#[tokio::test]
async fn codex_refresh_rejects_returned_subject_or_workspace_mismatch_without_writing() {
    for (label, subject, workspace) in [
        ("subject", "subject-b", "workspace-a"),
        ("workspace", "subject-a", "workspace-b"),
    ] {
        let directory = TemporaryDirectory::new(&format!("codex-refresh-mismatch-{label}"));
        let store = canonical_temp_path(directory.path().join("auth-store"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        seed_codex_connection(&store, &base, access_token(now_unix() - 1), now_unix() - 1);
        let before = std::fs::read_to_string(store.join("personal.connection")).unwrap();
        let server_base = base.clone();
        let server = thread::spawn(move || {
            let (request, stream) = receive_request(&listener);
            assert_refresh_request(&request);
            respond(
                stream,
                &json!({
                    "access_token": access_token(now_unix() + 3600),
                    "refresh_token": "synthetic-next-refresh",
                    "id_token": signed_identity(&server_base, subject, workspace)
                })
                .to_string(),
            );
            let (request, stream) = receive_request(&listener);
            assert!(request.starts_with("GET /discovery HTTP/1.1"));
            respond(
                stream,
                &json!({"issuer": format!("{server_base}/issuer"), "jwks_uri": format!("{server_base}/jwks")}).to_string(),
            );
            let (request, stream) = receive_request(&listener);
            assert!(request.starts_with("GET /jwks HTTP/1.1"));
            respond(stream, &jwks());
        });

        assert_eq!(
            service(&store, &base)
                .access_codex_credential()
                .await
                .unwrap_err()
                .to_string(),
            "subscription identity does not match the selected account"
        );
        server.join().unwrap();
        assert_eq!(
            std::fs::read_to_string(store.join("personal.connection")).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn codex_refresh_terminal_failure_clears_only_tokens() {
    let directory = TemporaryDirectory::new("codex-subscription-refresh-terminal");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let (base, server) = token_server(400, r#"{"error":"invalid_grant"}"#.to_owned());
    seed_codex_connection(&store, &base, access_token(now_unix() - 1), now_unix() - 1);

    assert_eq!(
        service(&store, &base)
            .access_codex_credential()
            .await
            .unwrap_err()
            .to_string(),
        "subscription credentials require login: HTTP 400: {\"error\":\"invalid_grant\"}"
    );
    server.join().unwrap();
    let stored = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(stored.contains("workspace-a"));
    assert!(stored.contains("\"tokens\":null"));
}

#[tokio::test]
async fn codex_refresh_treats_pinned_unauthorized_and_error_code_forms_as_terminal() {
    let cases = [
        (401, r#"{"error":"ignored"}"#.to_owned()),
        (
            500,
            r#"{"error":{"code":"REFRESH_TOKEN_INVALIDATED"}}"#.to_owned(),
        ),
        (500, r#"{"code":"refresh_token_reused"}"#.to_owned()),
        (500, r#"{"error":"refresh_token_expired"}"#.to_owned()),
    ];
    for (index, (status, body)) in cases.into_iter().enumerate() {
        let directory = TemporaryDirectory::new(&format!("codex-refresh-terminal-{index}"));
        let store = canonical_temp_path(directory.path().join("auth-store"));
        let (base, server) = token_server(status, body.clone());
        seed_codex_connection(&store, &base, access_token(now_unix() - 1), now_unix() - 1);

        assert_eq!(
            service(&store, &base)
                .access_codex_credential()
                .await
                .unwrap_err()
                .to_string(),
            format!("subscription credentials require login: HTTP {status}: {body}")
        );
        server.join().unwrap();
        assert!(
            std::fs::read_to_string(store.join("personal.connection"))
                .unwrap()
                .contains("\"tokens\":null")
        );
    }
}

#[tokio::test]
async fn codex_refresh_keeps_tokens_for_unknown_or_oversized_error_payloads() {
    let cases = [
        r#"{"error":{"code":"unknown_remote_code"}}"#.to_owned(),
        format!(
            r#"{{"error":"synthetic-remote-secret-{}"}}"#,
            "x".repeat(9 * 1024)
        ),
    ];
    for (index, body) in cases.into_iter().enumerate() {
        let directory = TemporaryDirectory::new(&format!("codex-refresh-nonterminal-{index}"));
        let store = canonical_temp_path(directory.path().join("auth-store"));
        let (base, server) = token_server(500, body.clone());
        seed_codex_connection(&store, &base, access_token(now_unix() - 1), now_unix() - 1);
        let before = std::fs::read_to_string(store.join("personal.connection")).unwrap();

        assert_eq!(
            service(&store, &base)
                .access_codex_credential()
                .await
                .unwrap_err()
                .to_string(),
            format!(
                "subscription authentication service is temporarily unavailable: HTTP 500: {}",
                if body.len() > 8 * 1024 {
                    "error body exceeds the configured read limit"
                } else {
                    &body
                }
            )
        );
        server.join().unwrap();
        assert_eq!(
            std::fs::read_to_string(store.join("personal.connection")).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn codex_refresh_rejects_an_access_token_without_a_numeric_expiry() {
    let directory = TemporaryDirectory::new("codex-subscription-refresh-expiry");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let (base, server) = token_server(200, r#"{"access_token":"e30.e30.signature"}"#.to_owned());
    seed_codex_connection(&store, &base, access_token(now_unix() - 1), now_unix() - 1);
    let before = std::fs::read_to_string(store.join("personal.connection")).unwrap();

    assert_eq!(
        service(&store, &base)
            .access_codex_credential()
            .await
            .unwrap_err()
            .to_string(),
        "subscription response is invalid"
    );
    server.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(store.join("personal.connection")).unwrap(),
        before
    );
}

#[tokio::test]
async fn codex_logout_uses_json_and_clears_local_tokens_when_remote_revoke_fails() {
    let directory = TemporaryDirectory::new("codex-subscription-revoke");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    seed_codex_connection(
        &store,
        &base,
        display_access_token(
            now_unix() + 3600,
            "logout@example.test",
            "Logout Name",
            "plus",
        ),
        now_unix() + 3600,
    );
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert!(request.starts_with("POST /revoke HTTP/1.1"));
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body,
            json!({
                "client_id": CLIENT_ID,
                "token": "synthetic-refresh",
                "token_type_hint": "refresh_token"
            })
        );
        respond_with_status(stream, 500, "{}");
    });

    let auth = service(&store, &base);
    let before = serde_json::to_value(auth.codex_status().await.unwrap()).unwrap();
    assert_eq!(before["email"], "logout@example.test");
    assert_eq!(
        serde_json::to_value(auth.logout().await.unwrap()).unwrap(),
        json!("remote_unconfirmed_cleared")
    );
    server.join().unwrap();
    let after = serde_json::to_value(auth.codex_status().await.unwrap()).unwrap();
    assert_eq!(after["state"], "login_required");
    assert_eq!(after["email"], Value::Null);
    assert_eq!(after["name"], Value::Null);
    assert_eq!(after["plan"], Value::Null);
    assert!(
        std::fs::read_to_string(store.join("personal.connection"))
            .unwrap()
            .contains("\"tokens\":null")
    );
}

#[tokio::test]
async fn codex_status_keeps_absent_or_nonstring_display_claims_optional() {
    let directory = TemporaryDirectory::new("codex-display-optional");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let base = "http://127.0.0.1:1";
    let token = format!(
        "e30.{}.signature",
        encode(&json!({
            "exp": now_unix() + 3600,
            "https://api.openai.com/profile": {"email": 123, "name": null},
            "https://api.openai.com/auth": {"chatgpt_plan_type": false}
        }))
    );
    seed_codex_connection(&store, base, token, now_unix() + 3600);
    let status = serde_json::to_value(service(&store, base).codex_status().await.unwrap()).unwrap();
    assert_eq!(status["state"], "connected");
    assert_eq!(status["email"], Value::Null);
    assert_eq!(status["name"], Value::Null);
    assert_eq!(status["plan"], Value::Null);
}

#[tokio::test]
async fn codex_status_ignores_unrelated_numbers_outside_json_value_range() {
    let directory = TemporaryDirectory::new("codex-display-unrepresentable");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let base = "http://127.0.0.1:1";
    let payload = format!(
        "{{\"exp\":{},\"unrelated\":1e999,\"https://api.openai.com/profile\":{{\"email\":\"valid@example.test\",\"name\":\"Valid Name\",\"unrelated\":1e999}},\"https://api.openai.com/auth\":{{\"chatgpt_plan_type\":\"plus\"}}}}",
        now_unix() + 3600
    );
    let token = format!(
        "e30.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    );
    seed_codex_connection(&store, base, token, now_unix() + 3600);
    let status = serde_json::to_value(service(&store, base).codex_status().await.unwrap()).unwrap();
    assert_eq!(status["state"], "connected");
    assert_eq!(status["email"], "valid@example.test");
    assert_eq!(status["name"], "Valid Name");
    assert_eq!(status["plan"], "plus");
}

#[tokio::test]
async fn codex_concurrent_refresh_rereads_the_single_stored_bearer_account_pair() {
    let directory = TemporaryDirectory::new("codex-subscription-concurrent-refresh");
    let store = canonical_temp_path(directory.path().join("auth-store"));
    let rotated_access = access_token(now_unix() + 3600);
    let (base, server) = token_server_with_no_followup(
        200,
        json!({"access_token": rotated_access.clone()}).to_string(),
    );
    seed_codex_connection(&store, &base, access_token(now_unix() - 1), now_unix() - 1);
    let first = service(&store, &base);
    let second = service(&store, &base);
    let (first, second) = tokio::join!(
        first.access_codex_credential(),
        second.access_codex_credential()
    );
    first.unwrap();
    second.unwrap();
    server.join().unwrap();
    let stored = std::fs::read_to_string(store.join("personal.connection")).unwrap();
    assert!(stored.contains(&rotated_access));
    assert!(stored.contains("workspace-a"));
}

fn send_callback(redirect_uri: &str, state: &str) {
    let address = redirect_uri
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET /auth/callback?state={state}&code=synthetic-code HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
}

fn token_server(status: u16, body: String) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert_refresh_request(&request);
        respond_with_status(stream, status, &body);
    });
    (base, server)
}

fn token_server_with_no_followup(status: u16, body: String) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (request, stream) = receive_request(&listener);
        assert_refresh_request(&request);
        respond_with_status(stream, status, &body);
        assert_no_followup_request(&listener);
    });
    (base, server)
}

fn assert_refresh_request(request: &str) {
    assert!(request.starts_with("POST /token HTTP/1.1"));
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        json!({
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": "synthetic-refresh"
        })
    );
}

fn assert_no_followup_request(listener: &TcpListener) {
    listener.set_nonblocking(true).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        match listener.accept() {
            Ok(_) => panic!("Codex refresh sent a second request instead of rereading storage"),
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

fn seed_codex_connection(store: &Path, base: &str, access_token: String, expires_at_unix: u64) {
    std::fs::create_dir(store).unwrap();
    set_private_directory(store);
    let connection = json!({
        "version": 2,
        "protocol": "codex",
        "account": "personal",
        "identity": {
            "client_id": CLIENT_ID,
            "subject": "subject-a",
            "issuer": format!("{base}/issuer"),
            "id_token_expires_at_unix": now_unix() + 3600,
            "chatgpt_account_id": "workspace-a"
        },
        "tokens": {
            "access_token": access_token,
            "refresh_token": "synthetic-refresh",
            "access_token_expires_at_unix": expires_at_unix
        }
    });
    let path = store.join("personal.connection");
    std::fs::write(&path, connection.to_string()).unwrap();
    set_private_file(&path);
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
                    "expected HTTP request"
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

fn respond(stream: TcpStream, body: &str) {
    respond_with_status(stream, 200, body);
}

fn respond_with_status(stream: TcpStream, status: u16, body: &str) {
    respond_with_status_and_header(stream, status, body, None);
}

fn respond_with_status_and_header(
    mut stream: TcpStream,
    status: u16,
    body: &str,
    header: Option<&str>,
) {
    let header = header
        .map(|value| format!("{value}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
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
            output.push((hex(bytes.next().unwrap()) << 4) | hex(bytes.next().unwrap()));
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

fn signed_identity(base: &str, subject: &str, account: &str) -> String {
    let now = now_unix();
    let header = encode(&json!({"alg":"RS256", "kid":TEST_KEY_ID}));
    let claims = encode(&json!({
        "iss": format!("{base}/issuer"),
        "aud": CLIENT_ID,
        "exp": now + 3600,
        "iat": now,
        "sub": subject,
        "https://api.openai.com/auth": {"chatgpt_account_id": account}
    }));
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

fn access_token(exp: u64) -> String {
    format!("e30.{}.signature", encode(&json!({"exp": exp})))
}

fn display_access_token(exp: u64, email: &str, name: &str, plan: &str) -> String {
    let payload = format!(
        "{{\"exp\":{exp},\"https://api.openai.com/profile\":{{}},\"https://api.openai.com/profile\":{},\"https://api.openai.com/auth\":{{}},\"https://api.openai.com/auth\":{}}}",
        json!({"email": email, "name": name}),
        json!({"chatgpt_plan_type": plan})
    );
    format!(
        "e30.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    )
}

fn encode(value: &Value) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap())
}

fn jwks() -> String {
    json!({"keys":[{
        "kid": TEST_KEY_ID,
        "kty": "RSA",
        "n": TEST_MODULUS,
        "e": "AQAB",
        "alg": "RS256",
        "use": "sig"
    }]})
    .to_string()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn canonical_temp_path(path: std::path::PathBuf) -> std::path::PathBuf {
    path.parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(path.file_name().unwrap())
}

#[cfg(unix)]
fn set_private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
fn set_private_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(not(unix))]
fn set_private_directory(_: &Path) {}

#[cfg(not(unix))]
fn set_private_file(_: &Path) {}
