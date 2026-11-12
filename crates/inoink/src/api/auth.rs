//! OAuth 2.0 authorization-code flow helpers and token storage contract.

use super::{Body, Clock, HttpRequest, Method, Transport};
use crate::error::{Error, Redacted, Result};
use std::fmt;
use url::form_urlencoded;

pub const AUTH_URL: &str = "https://www.inoreader.com/oauth2/auth";
pub const TOKEN_URL: &str = "https://www.inoreader.com/oauth2/token";

#[derive(Clone)]
pub struct AppConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub user_agent: String,
    /// Overridable for tests; defaults to the production reader API.
    pub base_url: String,
    pub token_url: String,
    /// Optional legacy headers. The OAuth docs do not require them.
    pub app_id: Option<String>,
    pub app_key: Option<String>,
}

impl AppConfig {
    pub fn new(client_id: &str, client_secret: &str, redirect_uri: &str) -> AppConfig {
        AppConfig {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            user_agent: format!("InoInk/{} (unofficial Kobo client)", crate::VERSION),
            base_url: "https://www.inoreader.com/reader/api/0".into(),
            token_url: TOKEN_URL.into(),
            app_id: None,
            app_key: None,
        }
    }
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &Redacted(&self.client_secret))
            .field("redirect_uri", &self.redirect_uri)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub scope: String,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &Redacted(&self.access_token))
            .field("refresh_token", &Redacted(&self.refresh_token))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Persistent token storage. `save` must be atomic (write-then-rename): a refreshed refresh
/// token is persisted before it is used anywhere else.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<Tokens>>;
    fn save(&self, tokens: &Tokens) -> Result<()>;
}

/// The consent page URL. `scope` is `read` or `read write`.
pub fn authorize_url(cfg: &AppConfig, state: &str, scope: &str) -> String {
    let q = form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", &cfg.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", scope)
        .append_pair("state", state)
        .finish();
    format!("{AUTH_URL}?{q}")
}

/// Extracts and validates `code` from a redirect callback URL or query string.
pub fn parse_callback(callback: &str, expected_state: &str) -> Result<String> {
    let query = callback.split_once('?').map(|(_, q)| q).unwrap_or(callback);
    let query = query.split('#').next().unwrap_or("");
    let mut code = None;
    let mut state = None;
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => return Err(Error::PermissionDenied("authorization was denied".into())),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err(Error::InvalidRequest("state mismatch".into()));
    }
    code.ok_or_else(|| Error::InvalidRequest("callback has no code".into()))
}

fn token_request(cfg: &AppConfig, pairs: &[(&str, &str)]) -> HttpRequest {
    let mut form: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    form.push(("client_id".into(), cfg.client_id.clone()));
    form.push(("client_secret".into(), cfg.client_secret.clone()));
    HttpRequest {
        method: Method::Post,
        url: cfg.token_url.clone(),
        headers: vec![("User-Agent".into(), cfg.user_agent.clone())],
        body: Some(Body::Form(form)),
    }
}

fn parse_tokens(status: u16, body: &[u8], now: i64, previous_refresh: Option<&str>) -> Result<Tokens> {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    if status == 400 || status == 401 {
        // invalid_grant, invalid_client, revoked: the user must authorize again.
        return Err(Error::NeedsAuthorization);
    }
    if !(200..300).contains(&status) {
        return Err(if status >= 500 {
            Error::Retryable(format!("token endpoint {status}"))
        } else {
            Error::InvalidRequest(format!("token endpoint {status}"))
        });
    }
    let access = v["access_token"].as_str().ok_or_else(|| Error::Data("token response has no access_token".into()))?;
    let refresh =
        v["refresh_token"].as_str().or(previous_refresh).ok_or_else(|| Error::Data("token response has no refresh_token".into()))?;
    let expires_in = v["expires_in"].as_i64().unwrap_or(3600);
    Ok(Tokens {
        access_token: access.into(),
        refresh_token: refresh.into(),
        expires_at: now + expires_in,
        scope: v["scope"].as_str().unwrap_or("").into(),
    })
}

pub fn exchange_code(cfg: &AppConfig, transport: &dyn Transport, clock: &dyn Clock, code: &str) -> Result<Tokens> {
    let req =
        token_request(cfg, &[("code", code), ("redirect_uri", &cfg.redirect_uri), ("scope", ""), ("grant_type", "authorization_code")]);
    let resp = transport.send(&req)?;
    parse_tokens(resp.status, &resp.body, clock.now(), None)
}

pub fn refresh(cfg: &AppConfig, transport: &dyn Transport, clock: &dyn Clock, refresh_token: &str) -> Result<Tokens> {
    let req = token_request(cfg, &[("grant_type", "refresh_token"), ("refresh_token", refresh_token)]);
    let resp = transport.send(&req)?;
    parse_tokens(resp.status, &resp.body, clock.now(), Some(refresh_token))
}
