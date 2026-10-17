use base64::Engine;
use ring::hmac;
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde::Deserialize;
use serde_json::Value;

use super::types::SubscriptionAuthError;

/// OpenID discovery fields used by the bounded token lifecycle. The adapter
/// accepts only signed `RS256` tokens; unsupported algorithms fail closed.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct OpenIdDiscovery {
    pub(super) issuer: String,
    pub(super) jwks_uri: String,
    pub(super) revocation_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct JsonWebKeySet {
    pub(super) keys: Vec<JsonWebKey>,
}

#[derive(Debug, Deserialize)]
pub(super) struct JsonWebKey {
    pub(super) kid: String,
    pub(super) kty: String,
    pub(super) n: String,
    pub(super) e: String,
    pub(super) alg: Option<String>,
    #[serde(rename = "use")]
    pub(super) key_use: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
    kid: String,
}

#[derive(Debug, Deserialize)]
struct IdentityClaims {
    iss: String,
    aud: Audience,
    exp: u64,
    nbf: Option<u64>,
    iat: Option<u64>,
    azp: Option<String>,
    sub: String,
}

pub(super) struct CodexIdentity {
    pub(super) subject: String,
    pub(super) issuer: String,
    pub(super) expires_at_unix: u64,
    pub(super) chatgpt_account_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn includes(&self, client_id: &str) -> bool {
        match self {
            Self::One(audience) => audience == client_id,
            Self::Many(audiences) => audiences.iter().any(|audience| audience == client_id),
        }
    }

    fn has_multiple(&self) -> bool {
        matches!(self, Self::Many(values) if values.len() > 1)
    }
}

pub(super) fn verify_identity_token(
    token: &str,
    discovery: &OpenIdDiscovery,
    keys: &JsonWebKeySet,
    client_id: &str,
    expected_nonce: &str,
    now_unix: u64,
) -> Result<String, SubscriptionAuthError> {
    let (claims, payload) = verify_signed_identity(token, discovery, keys, client_id, now_unix)?;
    let nonce = payload
        .get("nonce")
        .and_then(Value::as_str)
        .ok_or(SubscriptionAuthError::InvalidIdentityToken)?;
    if !constant_time_equal(nonce.as_bytes(), expected_nonce.as_bytes()) {
        return Err(SubscriptionAuthError::InvalidIdentityToken);
    }
    Ok(claims.sub)
}

pub(super) fn verify_codex_identity_token(
    token: &str,
    discovery: &OpenIdDiscovery,
    keys: &JsonWebKeySet,
    client_id: &str,
    now_unix: u64,
) -> Result<CodexIdentity, SubscriptionAuthError> {
    let (claims, payload) = verify_signed_identity(token, discovery, keys, client_id, now_unix)?;
    let chatgpt_account_id = payload
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|account| valid_chatgpt_account_id(account))
        .map(str::to_owned)
        .ok_or(SubscriptionAuthError::InvalidIdentityToken)?;
    Ok(CodexIdentity {
        subject: claims.sub,
        issuer: claims.iss,
        expires_at_unix: claims.exp,
        chatgpt_account_id,
    })
}

fn verify_signed_identity(
    token: &str,
    discovery: &OpenIdDiscovery,
    keys: &JsonWebKeySet,
    client_id: &str,
    now_unix: u64,
) -> Result<(IdentityClaims, Value), SubscriptionAuthError> {
    let mut segments = token.split('.');
    let header_segment = segments
        .next()
        .ok_or(SubscriptionAuthError::InvalidIdentityToken)?;
    let claims_segment = segments
        .next()
        .ok_or(SubscriptionAuthError::InvalidIdentityToken)?;
    let signature_segment = segments
        .next()
        .ok_or(SubscriptionAuthError::InvalidIdentityToken)?;
    if segments.next().is_some()
        || header_segment.is_empty()
        || claims_segment.is_empty()
        || signature_segment.is_empty()
    {
        return Err(SubscriptionAuthError::InvalidIdentityToken);
    }

    let header: JwtHeader = decode_json(header_segment)?;
    if header.alg != "RS256" || header.kid.is_empty() {
        return Err(SubscriptionAuthError::InvalidIdentityToken);
    }
    let key = keys
        .keys
        .iter()
        .find(|key| {
            key.kid == header.kid
                && key.kty == "RSA"
                && key
                    .alg
                    .as_deref()
                    .is_none_or(|algorithm| algorithm == "RS256")
                && key
                    .key_use
                    .as_deref()
                    .is_none_or(|key_use| key_use == "sig")
        })
        .ok_or(SubscriptionAuthError::InvalidIdentityToken)?;
    let signature = decode_base64(signature_segment)?;
    let signed = format!("{header_segment}.{claims_segment}");
    let (modulus, exponent) = rsa_components(key)?;
    RsaPublicKeyComponents {
        n: &modulus,
        e: &exponent,
    }
    .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
    .map_err(|_| SubscriptionAuthError::InvalidIdentityToken)?;

    let payload: Value = decode_json(claims_segment)?;
    let claims: IdentityClaims = serde_json::from_value(payload.clone())
        .map_err(|_| SubscriptionAuthError::InvalidIdentityToken)?;
    if claims.iss != discovery.issuer
        || !claims.aud.includes(client_id)
        || claims.azp.as_deref().is_some_and(|azp| azp != client_id)
        || (claims.aud.has_multiple() && claims.azp.as_deref() != Some(client_id))
        || claims.exp <= now_unix
        || claims.nbf.is_some_and(|not_before| not_before > now_unix)
        || claims
            .iat
            .is_some_and(|issued_at| issued_at > now_unix.saturating_add(60))
        || claims.sub.is_empty()
        || claims.sub.len() > 512
    {
        return Err(SubscriptionAuthError::InvalidIdentityToken);
    }
    Ok((claims, payload))
}

pub(super) fn valid_chatgpt_account_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

pub(super) fn constant_time_equal(expected: &[u8], candidate: &[u8]) -> bool {
    let expected_key = hmac::Key::new(hmac::HMAC_SHA256, expected);
    let candidate_key = hmac::Key::new(hmac::HMAC_SHA256, candidate);
    let candidate_tag = hmac::sign(&candidate_key, b"subscription-auth-equality");
    hmac::verify(
        &expected_key,
        b"subscription-auth-equality",
        candidate_tag.as_ref(),
    )
    .is_ok()
}

fn decode_json<T: for<'de> Deserialize<'de>>(segment: &str) -> Result<T, SubscriptionAuthError> {
    let bytes = decode_base64(segment)?;
    serde_json::from_slice(&bytes).map_err(|_| SubscriptionAuthError::InvalidIdentityToken)
}

fn decode_base64(segment: &str) -> Result<Vec<u8>, SubscriptionAuthError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(segment)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(segment))
        .map_err(|_| SubscriptionAuthError::InvalidIdentityToken)
}

fn rsa_components(key: &JsonWebKey) -> Result<(Vec<u8>, Vec<u8>), SubscriptionAuthError> {
    let modulus = decode_base64(&key.n)?;
    let exponent = decode_base64(&key.e)?;
    if !(256..=1024).contains(&modulus.len()) || exponent.is_empty() || exponent.len() > 8 {
        return Err(SubscriptionAuthError::InvalidIdentityToken);
    }
    Ok((modulus, exponent))
}
