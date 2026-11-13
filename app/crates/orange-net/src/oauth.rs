//! OAuth 2.0 for HTTP connections: getting an access token from a token endpoint,
//! keeping it until shortly before it expires, and getting another.
//!
//! Two grants are supported, both without a person present: client credentials, and a
//! refresh token obtained earlier (for example with `orange oauth login`, which runs
//! the authorization-code flow with PKCE in a browser). A token endpoint must be
//! `https://`, except on this machine (`localhost`, `127.0.0.1`), where a local
//! identity provider or a test may listen without TLS.
use serde_json::{Map, Value as Json};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};
use ureq::Agent;
use ureq::http::Request;

/// Largest token response read.
const MAX_TOKEN_BYTES: u64 = 64 * 1024;
/// A token is replaced this long before it expires, so a request never carries one
/// that expires on the way.
const EARLY: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(30);

fn form_encode(pairs: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, value) in pairs {
        if !out.is_empty() {
            out.push('&');
        }
        for (index, part) in [name, value].into_iter().enumerate() {
            if index == 1 {
                out.push('=');
            }
            for byte in part.bytes() {
                if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                    out.push(char::from(byte));
                } else {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

/// Whether a token or authorization endpoint may be used: `https://`, or `http://` to
/// this machine only.
pub fn is_endpoint(url: &str) -> bool {
    let local = url.strip_prefix("http://").is_some_and(|rest| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit_once(':').map_or(host, |(host, _)| host);
        matches!(host, "localhost" | "127.0.0.1")
    });
    (url.starts_with("https://") || local)
        && url.len() <= 2048
        && url.chars().all(|c| c.is_ascii_graphic())
        && !url.contains('@')
}

/// What a token endpoint answered.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    /// Seconds the access token is valid for, when the server says.
    pub expires_in: Option<u64>,
    /// A refresh token, when the server issued or replaced one.
    pub refresh_token: Option<String>,
}
/// The tokens are left out.
impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("expires_in", &self.expires_in)
            .field("refresh_token", &self.refresh_token.is_some())
            .finish_non_exhaustive()
    }
}

/// Sends one request to a token endpoint: `form` as the body and, with `basic`, the
/// client's ID and secret as HTTP Basic authorization instead of form fields. Errors
/// say what failed without repeating anything that was sent or received.
pub fn token_request(
    token_url: &str,
    form: &[(&str, &str)],
    basic: Option<(&str, &str)>,
) -> Result<TokenResponse, String> {
    if !is_endpoint(token_url) {
        return Err("the token URL must be https://, or http:// on this machine".into());
    }
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut request = Request::builder()
        .method("POST")
        .uri(token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json");
    if let Some((id, secret)) = basic {
        let pair = format!("{id}:{secret}");
        request = request.header(
            "Authorization",
            format!("Basic {}", crate::base64(pair.as_bytes())),
        );
    }
    let request = request
        .body(form_encode(form).into_bytes())
        .map_err(|_| "the token request is not valid".to_owned())?;
    let mut response = agent
        .run(request)
        .map_err(|_| "the token endpoint could not be reached".to_owned())?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_TOKEN_BYTES)
        .read_to_vec()
        .map_err(|_| "the token response is too large or was cut short".to_owned())?;
    let answer: Json = serde_json::from_slice(&body).unwrap_or(Json::Null);
    if status != 200 {
        // The error code is one of a fixed set the standard defines; nothing else of
        // the answer is repeated.
        let code = answer["error"]
            .as_str()
            .filter(|code| {
                code.len() <= 64 && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            })
            .unwrap_or("no error code");
        return Err(format!(
            "the token endpoint refused with status {status} ({code})"
        ));
    }
    if answer["token_type"]
        .as_str()
        .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
    {
        return Err("the token endpoint issued a token that is not a bearer token".into());
    }
    let access_token = answer["access_token"]
        .as_str()
        .filter(|token| !token.is_empty() && !token.chars().any(|c| c.is_control() || c == ' '))
        .ok_or("the token endpoint issued no access token")?
        .to_owned();
    Ok(TokenResponse {
        access_token,
        expires_in: answer["expires_in"].as_u64(),
        refresh_token: answer["refresh_token"].as_str().map(str::to_owned),
    })
}

/// The `oauth` setting of a connection and the token it currently holds.
pub(crate) struct OAuth {
    token_url: String,
    client_id: String,
    client_secret: Option<String>,
    scope: Option<String>,
    /// Send the client's ID and secret as Basic authorization, not as form fields.
    basic: bool,
    /// The refresh token, replaced when the server rotates it. `None` selects the
    /// client-credentials grant.
    refresh: Mutex<Option<String>>,
    token: Mutex<Option<(String, Option<Instant>)>>,
}
/// Secrets and tokens are left out.
impl std::fmt::Debug for OAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth")
            .field("token_url", &self.token_url)
            .finish_non_exhaustive()
    }
}

impl OAuth {
    /// Reads the `oauth` setting. Error text names the field, never its value.
    pub(crate) fn from_setting(setting: &Json) -> Result<Self, String> {
        let fields: &Map<String, Json> = setting.as_object().ok_or("`oauth` must be an object")?;
        for name in fields.keys() {
            if !matches!(
                name.as_str(),
                "token_url"
                    | "client_id"
                    | "client_secret"
                    | "refresh_token"
                    | "scope"
                    | "client_auth"
            ) {
                return Err(format!("`oauth` has an unknown field `{name}`"));
            }
        }
        let text = |name: &str| match fields.get(name) {
            None => Ok(None),
            Some(Json::String(value))
                if !value.is_empty() && !value.chars().any(char::is_control) =>
            {
                Ok(Some(value.clone()))
            }
            Some(_) => Err(format!("`oauth.{name}` must be text")),
        };
        let token_url = text("token_url")?.ok_or("`oauth.token_url` is required")?;
        if !is_endpoint(&token_url) {
            return Err("`oauth.token_url` must be https://, or http:// on this machine".into());
        }
        let client_secret = text("client_secret")?;
        let refresh = text("refresh_token")?;
        if client_secret.is_none() && refresh.is_none() {
            return Err("`oauth` needs `client_secret` or `refresh_token`".into());
        }
        let basic = match text("client_auth")?.as_deref() {
            None | Some("body") => false,
            Some("basic") if client_secret.is_some() => true,
            Some(_) => {
                return Err("`oauth.client_auth` must be `body`, or `basic` with a secret".into());
            }
        };
        Ok(Self {
            token_url,
            client_id: text("client_id")?.ok_or("`oauth.client_id` is required")?,
            client_secret,
            scope: text("scope")?,
            basic,
            refresh: Mutex::new(refresh),
            token: Mutex::new(None),
        })
    }

    /// The access token to send. With `renew`, the one held is given up first, as
    /// after a request the server answered with 401.
    pub(crate) fn token(&self, renew: bool) -> Result<String, String> {
        // Held across the request: several nodes waiting for one token ask for it once.
        let mut held = self.token.lock().unwrap_or_else(PoisonError::into_inner);
        if !renew
            && let Some((token, expires)) = held.as_ref()
            && expires.is_none_or(|expires| Instant::now() + EARLY < expires)
        {
            return Ok(token.clone());
        }
        let refresh = self
            .refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut form: Vec<(&str, &str)> = Vec::new();
        match &refresh {
            Some(refresh) => {
                form.push(("grant_type", "refresh_token"));
                form.push(("refresh_token", refresh));
            }
            None => form.push(("grant_type", "client_credentials")),
        }
        if let Some(scope) = &self.scope {
            form.push(("scope", scope));
        }
        let basic = match (&self.client_secret, self.basic) {
            (Some(secret), true) => Some((self.client_id.as_str(), secret.as_str())),
            (secret, _) => {
                form.push(("client_id", &self.client_id));
                if let Some(secret) = secret {
                    form.push(("client_secret", secret));
                }
                None
            }
        };
        let answer = token_request(&self.token_url, &form, basic)?;
        // Only a connection set up with a refresh token follows its rotation; one
        // using client credentials keeps that grant whatever the server sends.
        if let Some(rotated) = answer.refresh_token.filter(|_| refresh.is_some()) {
            *self.refresh.lock().unwrap_or_else(PoisonError::into_inner) = Some(rotated);
        }
        let expires = answer
            .expires_in
            .map(|seconds| Instant::now() + Duration::from_secs(seconds));
        *held = Some((answer.access_token.clone(), expires));
        Ok(answer.access_token)
    }
}

/// A random value for an authorization request's `state` or PKCE verifier: 64
/// hexadecimal digits.
pub fn random_token() -> Result<String, String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "no random numbers are available".to_owned())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The PKCE `S256` challenge of a verifier: the URL-safe, unpadded base64 of its
/// SHA-256.
pub fn pkce_challenge(verifier: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
    crate::base64(digest.as_ref())
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// The URL a person opens to authorize: the authorization endpoint with the request's
/// parameters added.
pub fn authorization_url(authorize_url: &str, parameters: &[(&str, &str)]) -> String {
    let separator = if authorize_url.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{authorize_url}{separator}{}", form_encode(parameters))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_encoded_and_endpoints_are_checked() {
        assert_eq!(
            form_encode(&[("scope", "read write"), ("a&b", "c=d/é")]),
            "scope=read%20write&a%26b=c%3Dd%2F%C3%A9"
        );
        // The example of RFC 7636, appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(
            authorization_url("https://id.example/authorize", &[("state", "x y")]),
            "https://id.example/authorize?state=x%20y"
        );
        for good in [
            "https://id.example/token",
            "http://localhost:8080/token",
            "http://127.0.0.1/t",
        ] {
            assert!(is_endpoint(good), "{good}");
        }
        for bad in [
            "http://id.example/token",
            "http://localhost.evil.example/t",
            "ftp://x",
            "https://u@id.example/",
        ] {
            assert!(!is_endpoint(bad), "{bad}");
        }
        let token = random_token().unwrap();
        assert_eq!(token.len(), 64);
        assert_ne!(token, random_token().unwrap());
    }

    #[test]
    fn the_oauth_setting_is_checked_without_repeating_secrets() {
        let parse = |text: &str| OAuth::from_setting(&serde_json::from_str(text).unwrap());
        assert!(
            parse(
                r#"{"token_url": "https://id.example/t", "client_id": "app", "client_secret": "s"}"#
            )
            .is_ok()
        );
        assert!(
            parse(
                r#"{"token_url": "https://id.example/t", "client_id": "app", "refresh_token": "r"}"#
            )
            .is_ok()
        );
        for bad in [
            r#"{"client_id": "app", "client_secret": "s"}"#,
            r#"{"token_url": "http://id.example/t", "client_id": "app", "client_secret": "s"}"#,
            r#"{"token_url": "https://id.example/t", "client_secret": "s"}"#,
            r#"{"token_url": "https://id.example/t", "client_id": "app"}"#,
            r#"{"token_url": "https://id.example/t", "client_id": "app", "refresh_token": "r", "client_auth": "basic"}"#,
            r#"{"token_url": "https://id.example/t", "client_id": "app", "client_secret": "s", "password": "p"}"#,
            r#"[]"#,
        ] {
            let error = parse(bad).unwrap_err();
            assert!(!error.contains("\"s\""), "{error}");
        }
        let debug = format!("{:?}", parse(r#"{"token_url": "https://id.example/t", "client_id": "app", "client_secret": "generated-secret"}"#).unwrap());
        assert!(!debug.contains("generated-secret"));
    }
}
