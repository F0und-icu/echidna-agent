#[allow(dead_code)]
#[path = "../src/adapters/subscription_auth/jwt.rs"]
mod jwt;
#[allow(dead_code)]
#[path = "../src/adapters/subscription_auth/types.rs"]
mod types;

use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::{Value, json};

const MODULUS: &str = "ytNu3Ek5u84iuxLHVotlIugx4KbTo9xtN_AbeosoA3b8fe9_7EX1j8Hah6iLi7FI5_jn1cCEVLCepoXWV-R-P3MVigqdoAiClX2sVmLhyDZ57p8nK1iQSBTeuklOHXYMk5GKTL43P3-cJ8O5V-lSj4CYACrQxV4_B0tbwxAZOj3H98Jog52FhhO5xZ6AfNadTbIvIbWSnVZ06TmAUSie8T1jeaB-2ABVB_GtVshazXPo7T-rFJlWF2kOqDy4e7p5oNxD1SgbN_IUTL8iqUgnb74ySO_ZFcugI_gUywIb2_l38VlzLLZ1XXURumFrlTyjVxDC7zx_hJq0eLjiZgpvBQ";

fn claims() -> Value {
    json!({"iss":"https://issuer.test", "aud":"issued-client", "exp":2000,
        "nbf":900, "iat":900, "nonce":"expected-nonce", "sub":"account-subject"})
}

fn signed(header: Value, claims: Value) -> String {
    let encode = |value: &Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap())
    };
    let message = format!("{}.{}", encode(&header), encode(&claims));
    let key = RsaKeyPair::from_pkcs8(include_bytes!(
        "fixtures/subscription_auth/test-rsa-2048.pk8"
    ))
    .unwrap();
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        message.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!(
        "{message}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    )
}

fn verify(token: &str) -> Result<String, types::SubscriptionAuthError> {
    let discovery = serde_json::from_value(json!({
        "issuer":"https://issuer.test", "jwks_uri":"https://issuer.test/jwks"
    }))
    .unwrap();
    let keys = serde_json::from_value(json!({"keys":[{
        "kid":"synthetic", "kty":"RSA", "n":MODULUS, "e":"AQAB",
        "alg":"RS256", "use":"sig"
    }]}))
    .unwrap();
    jwt::verify_identity_token(
        token,
        &discovery,
        &keys,
        "issued-client",
        "expected-nonce",
        1000,
    )
}

#[test]
fn signed_identity_accepts_single_audience_and_explicit_multiple_audience_party() {
    for audience in [json!("issued-client"), json!(["issued-client", "other"])] {
        let mut claims = claims();
        claims["aud"] = audience;
        claims["azp"] = json!("issued-client");
        let token = signed(json!({"alg":"RS256", "kid":"synthetic"}), claims);
        assert_eq!(verify(&token).unwrap(), "account-subject");
    }
}

#[test]
fn signed_identity_rejects_invalid_issuer_audience_time_nonce_and_subject() {
    let cases = [
        ("issuer", "iss", json!("https://other.test")),
        ("audience", "aud", json!("other-client")),
        ("empty-audience", "aud", json!([])),
        ("missing-party", "aud", json!(["issued-client", "other"])),
        ("wrong-party", "azp", json!("other-client")),
        ("expiry-boundary", "exp", json!(1000)),
        ("future-not-before", "nbf", json!(1001)),
        ("future-issued-at", "iat", json!(1061)),
        ("nonce", "nonce", json!("other-nonce")),
        ("empty-subject", "sub", json!("")),
        ("oversized-subject", "sub", json!("x".repeat(513))),
    ];
    for (label, field, value) in cases {
        let mut claims = claims();
        claims[field] = value;
        let token = signed(json!({"alg":"RS256", "kid":"synthetic"}), claims);
        assert_eq!(
            verify(&token),
            Err(types::SubscriptionAuthError::InvalidIdentityToken),
            "{label}"
        );
    }
    for field in ["iss", "aud", "exp", "nonce", "sub"] {
        let mut claims = claims();
        claims.as_object_mut().unwrap().remove(field);
        let token = signed(json!({"alg":"RS256", "kid":"synthetic"}), claims);
        assert_eq!(
            verify(&token),
            Err(types::SubscriptionAuthError::InvalidIdentityToken),
            "missing {field}"
        );
    }
}

#[test]
fn identity_rejects_tampering_unknown_keys_algorithms_and_extra_segments() {
    for header in [
        json!({"alg":"none", "kid":"synthetic"}),
        json!({"alg":"HS256", "kid":"synthetic"}),
        json!({"alg":"RS256", "kid":"unknown"}),
    ] {
        assert_eq!(
            verify(&signed(header, claims())),
            Err(types::SubscriptionAuthError::InvalidIdentityToken)
        );
    }
    let token = signed(json!({"alg":"RS256", "kid":"synthetic"}), claims());
    let (message, signature) = token.rsplit_once('.').unwrap();
    let mut signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(signature)
        .unwrap();
    signature[0] ^= 1;
    let tampered = format!(
        "{message}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    );
    assert_eq!(
        verify(&tampered),
        Err(types::SubscriptionAuthError::InvalidIdentityToken)
    );
    assert_eq!(
        verify(&format!("{token}.extra")),
        Err(types::SubscriptionAuthError::InvalidIdentityToken)
    );
}
