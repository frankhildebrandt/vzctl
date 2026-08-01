//! Minimal OIDC Authorization Code + PKCE IdP with session cookie + user picker.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use rand::rngs::OsRng;
use rsa::pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey};
use rsa::pkcs8::LineEnding;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

const SESSION_COOKIE: &str = "vzctl_oidc_simple";
const CODE_TTL_SECS: u64 = 300;
const TOKEN_TTL_SECS: u64 = 3600;
const SESSION_TTL_SECS: u64 = 86400 * 7;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub issuer: String,
    pub listen: String,
    pub clients: Vec<ClientConfig>,
    pub users: Vec<UserConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientConfig {
    pub id: String,
    pub secret: String,
    #[serde(rename = "redirectURIs", alias = "redirect_uris")]
    pub redirect_uris: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserConfig {
    pub username: String,
    pub email: String,
    #[serde(default)]
    pub claims: HashMap<String, Value>,
}

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<Store>>,
    config: Arc<ProviderConfig>,
    encoding_key: EncodingKey,
    jwks: Value,
    kid: String,
}

struct Store {
    sessions: HashMap<String, Session>,
    codes: HashMap<String, AuthCode>,
    access_tokens: HashMap<String, TokenRecord>,
}

#[derive(Clone)]
struct Session {
    username: String,
    expires_at: u64,
}

#[derive(Clone)]
struct AuthCode {
    username: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    nonce: Option<String>,
    scope: String,
    expires_at: u64,
}

#[derive(Clone)]
struct TokenRecord {
    username: String,
    #[allow(dead_code)]
    client_id: String,
    #[allow(dead_code)]
    scope: String,
    expires_at: u64,
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    scope: Option<String>,
    state: Option<String>,
    nonce: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    client_id: String,
    redirect_uri: String,
    scope: String,
    state: Option<String>,
    nonce: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
}

#[derive(Deserialize)]
struct TokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    code_verifier: Option<String>,
}

#[derive(Deserialize)]
struct EndSessionQuery {
    post_logout_redirect_uri: Option<String>,
    client_id: Option<String>,
    state: Option<String>,
}

#[derive(Serialize)]
struct IdTokenClaims {
    iss: String,
    sub: String,
    aud: String,
    exp: u64,
    iat: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    preferred_username: String,
    email: String,
    email_verified: bool,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

pub fn load_config(path: &Path) -> Result<ProviderConfig, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let cfg: ProviderConfig =
        serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
    if cfg.issuer.is_empty() {
        return Err("issuer must not be empty".into());
    }
    if cfg.users.is_empty() {
        return Err("users must not be empty".into());
    }
    if cfg.listen.is_empty() {
        return Err("listen must not be empty".into());
    }
    Ok(cfg)
}

pub async fn run(config: ProviderConfig, work_dir: Option<PathBuf>) -> Result<(), String> {
    let work_dir = work_dir.unwrap_or_else(|| PathBuf::from("."));
    let (encoding_key, jwks, kid) = load_or_create_keys(&work_dir)?;
    let state = AppState {
        inner: Arc::new(Mutex::new(Store {
            sessions: HashMap::new(),
            codes: HashMap::new(),
            access_tokens: HashMap::new(),
        })),
        config: Arc::new(config.clone()),
        encoding_key,
        jwks,
        kid,
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks_handler))
        .route("/authorize", get(authorize))
        .route("/auth", get(authorize))
        .route("/login", post(login))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo).post(userinfo))
        .route("/end_session", get(end_session))
        .route("/logout", get(end_session))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .map_err(|e| format!("bind {}: {e}", config.listen))?;
    tracing::info!(
        listen = %config.listen,
        issuer = %config.issuer,
        users = config.users.len(),
        "vzctl-oidc-simple listening"
    );
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("serve: {e}"))
}

fn load_or_create_keys(work_dir: &Path) -> Result<(EncodingKey, Value, String), String> {
    let key_path = work_dir.join("signing.pem");
    let pem = if key_path.exists() {
        std::fs::read_to_string(&key_path)
            .map_err(|e| format!("read {}: {e}", key_path.display()))?
    } else {
        let mut rng = OsRng;
        let private =
            RsaPrivateKey::new(&mut rng, 2048).map_err(|e| format!("generate rsa key: {e}"))?;
        let pem = private
            .to_pkcs1_pem(LineEnding::LF)
            .map_err(|e| format!("encode pem: {e}"))?
            .to_string();
        std::fs::write(&key_path, &pem)
            .map_err(|e| format!("write {}: {e}", key_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }
        pem
    };
    let private =
        RsaPrivateKey::from_pkcs1_pem(&pem).map_err(|e| format!("parse signing key: {e}"))?;
    let encoding_key =
        EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| format!("jwt key: {e}"))?;
    let kid = {
        let mut hasher = Sha256::new();
        hasher.update(pem.as_bytes());
        format!("{:x}", hasher.finalize())[..16].to_string()
    };
    let n = URL_SAFE_NO_PAD.encode(private.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(private.e().to_bytes_be());
    let jwks = json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": kid,
            "n": n,
            "e": e,
        }]
    });
    Ok((encoding_key, jwks, kid))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn user_sub(username: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"vzctl-oidc-simple:");
    hasher.update(username.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn find_user<'a>(config: &'a ProviderConfig, username: &str) -> Option<&'a UserConfig> {
    config.users.iter().find(|u| u.username == username)
}

fn find_client<'a>(config: &'a ProviderConfig, id: &str) -> Option<&'a ClientConfig> {
    config.clients.iter().find(|c| c.id == id)
}

fn parse_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn set_session_cookie(sid: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={sid}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECS}"
    ))
    .expect("cookie header")
}

fn clear_session_cookie() -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"
    ))
    .expect("cookie header")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn root(State(state): State<AppState>) -> Html<String> {
    Html(root_html(&state.config))
}

fn root_html(config: &ProviderConfig) -> String {
    let issuer = html_escape(config.issuer.trim_end_matches('/'));
    let mut users = String::new();
    for user in &config.users {
        let username = html_escape(&user.username);
        let email = html_escape(&user.email);
        users.push_str(&format!(
            r#"<li><strong>{username}</strong> <span>{email}</span></li>"#
        ));
    }
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1"/>
  <title>vzctl oidc-simple</title>
  <style>
    :root {{ color-scheme: light dark; font-family: ui-sans-serif, system-ui, sans-serif; }}
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center;
      background: radial-gradient(circle at top, #1e293b, #0f172a); color: #e2e8f0; }}
    main {{ width: min(28rem, 92vw); background: #111827; border: 1px solid #334155;
      border-radius: 12px; padding: 1.5rem; box-shadow: 0 20px 50px rgba(0,0,0,.35); }}
    h1 {{ margin: 0 0 .25rem; font-size: 1.25rem; }}
    p {{ margin: 0 0 1rem; color: #94a3b8; font-size: .9rem; }}
    a {{ color: #38bdf8; }}
    ul {{ margin: 0 0 1rem; padding-left: 1.1rem; color: #cbd5e1; }}
    li {{ margin: .35rem 0; }}
    li span {{ color: #94a3b8; font-size: .85rem; }}
    code {{ font-size: .8rem; color: #93c5fd; word-break: break-all; }}
    .badge {{ display: inline-block; font-size: .7rem; letter-spacing: .04em; text-transform: uppercase;
      color: #fbbf24; border: 1px solid #92400e; border-radius: 999px; padding: .1rem .5rem; margin-bottom: .75rem; }}
  </style>
</head>
<body>
  <main>
    <div class="badge">dev only</div>
    <h1>oidc-simple</h1>
    <p>Dev IdP is up. Login starts from your app via <code>/authorize</code> (Auth Code + PKCE).</p>
    <p>Issuer: <code>{issuer}</code></p>
    <p><a href="/.well-known/openid-configuration">OpenID discovery</a></p>
    <p>Configured users:</p>
    <ul>{users}</ul>
  </main>
</body>
</html>"#
    )
}

fn picker_html(config: &ProviderConfig, q: &AuthorizeQuery) -> String {
    let client_id = html_escape(q.client_id.as_deref().unwrap_or(""));
    let redirect_uri = html_escape(q.redirect_uri.as_deref().unwrap_or(""));
    let scope = html_escape(q.scope.as_deref().unwrap_or("openid profile email"));
    let state = html_escape(q.state.as_deref().unwrap_or(""));
    let nonce = html_escape(q.nonce.as_deref().unwrap_or(""));
    let code_challenge = html_escape(q.code_challenge.as_deref().unwrap_or(""));
    let code_challenge_method = html_escape(q.code_challenge_method.as_deref().unwrap_or(""));

    let mut buttons = String::new();
    for user in &config.users {
        let username = html_escape(&user.username);
        let email = html_escape(&user.email);
        buttons.push_str(&format!(
            r#"<button type="submit" name="username" value="{username}" class="user">
  <strong>{username}</strong>
  <span>{email}</span>
</button>"#
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1"/>
  <title>vzctl oidc-simple</title>
  <style>
    :root {{ color-scheme: light dark; font-family: ui-sans-serif, system-ui, sans-serif; }}
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center;
      background: radial-gradient(circle at top, #1e293b, #0f172a); color: #e2e8f0; }}
    main {{ width: min(28rem, 92vw); background: #111827; border: 1px solid #334155;
      border-radius: 12px; padding: 1.5rem; box-shadow: 0 20px 50px rgba(0,0,0,.35); }}
    h1 {{ margin: 0 0 .25rem; font-size: 1.25rem; }}
    p {{ margin: 0 0 1rem; color: #94a3b8; font-size: .9rem; }}
    form {{ display: grid; gap: .6rem; }}
    .user {{ display: flex; flex-direction: column; align-items: flex-start; gap: .15rem;
      width: 100%; text-align: left; cursor: pointer; border: 1px solid #475569;
      background: #0f172a; color: inherit; border-radius: 8px; padding: .75rem .9rem; }}
    .user:hover {{ border-color: #38bdf8; background: #172554; }}
    .user strong {{ font-size: 1rem; }}
    .user span {{ color: #94a3b8; font-size: .85rem; }}
    .badge {{ display: inline-block; font-size: .7rem; letter-spacing: .04em; text-transform: uppercase;
      color: #fbbf24; border: 1px solid #92400e; border-radius: 999px; padding: .1rem .5rem; margin-bottom: .75rem; }}
  </style>
</head>
<body>
  <main>
    <div class="badge">dev only</div>
    <h1>Pick a user</h1>
    <p>oidc-simple — no passwords. Click to sign in.</p>
    <form method="post" action="/login">
      <input type="hidden" name="client_id" value="{client_id}"/>
      <input type="hidden" name="redirect_uri" value="{redirect_uri}"/>
      <input type="hidden" name="scope" value="{scope}"/>
      <input type="hidden" name="state" value="{state}"/>
      <input type="hidden" name="nonce" value="{nonce}"/>
      <input type="hidden" name="code_challenge" value="{code_challenge}"/>
      <input type="hidden" name="code_challenge_method" value="{code_challenge_method}"/>
      {buttons}
    </form>
  </main>
</body>
</html>"#
    )
}

async fn discovery(State(state): State<AppState>) -> Json<Value> {
    let issuer = state.config.issuer.trim_end_matches('/');
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "jwks_uri": format!("{issuer}/jwks"),
        "end_session_endpoint": format!("{issuer}/end_session"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "scopes_supported": ["openid", "profile", "email"],
        "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic", "none"],
        "code_challenge_methods_supported": ["S256", "plain"],
        "claims_supported": ["sub", "iss", "aud", "exp", "iat", "preferred_username", "email", "email_verified"],
    }))
}

async fn jwks_handler(State(state): State<AppState>) -> Json<Value> {
    Json(state.jwks.clone())
}

async fn authorize(
    State(state): State<AppState>,
    Query(q): Query<AuthorizeQuery>,
    headers: HeaderMap,
) -> Response {
    if q.response_type.as_deref() != Some("code") {
        return (StatusCode::BAD_REQUEST, "response_type must be code").into_response();
    }
    let Some(client_id) = q.client_id.as_deref() else {
        return (StatusCode::BAD_REQUEST, "client_id required").into_response();
    };
    let Some(redirect_uri) = q.redirect_uri.as_deref() else {
        return (StatusCode::BAD_REQUEST, "redirect_uri required").into_response();
    };
    let Some(client) = find_client(&state.config, client_id) else {
        return (StatusCode::BAD_REQUEST, "unknown client_id").into_response();
    };
    if !client.redirect_uris.iter().any(|u| u == redirect_uri) {
        return (StatusCode::BAD_REQUEST, "redirect_uri not registered").into_response();
    }

    let sid = parse_cookie(&headers, SESSION_COOKIE);
    if let Some(sid) = sid {
        let mut store = state.inner.lock().await;
        if let Some(session) = store.sessions.get(&sid).cloned() {
            if session.expires_at > now() && find_user(&state.config, &session.username).is_some() {
                let code = issue_code(&mut store, &session.username, client_id, redirect_uri, &q);
                return Redirect::temporary(&build_redirect(
                    redirect_uri,
                    &code,
                    q.state.as_deref(),
                ))
                .into_response();
            }
            store.sessions.remove(&sid);
        }
    }

    Html(picker_html(&state.config, &q)).into_response()
}

fn issue_code(
    store: &mut Store,
    username: &str,
    client_id: &str,
    redirect_uri: &str,
    q: &AuthorizeQuery,
) -> String {
    let code = Uuid::new_v4().to_string();
    store.codes.insert(
        code.clone(),
        AuthCode {
            username: username.to_string(),
            client_id: client_id.to_string(),
            redirect_uri: redirect_uri.to_string(),
            code_challenge: q.code_challenge.clone(),
            code_challenge_method: q.code_challenge_method.clone(),
            nonce: q.nonce.clone(),
            scope: q
                .scope
                .clone()
                .unwrap_or_else(|| "openid profile email".into()),
            expires_at: now() + CODE_TTL_SECS,
        },
    );
    code
}

fn build_redirect(redirect_uri: &str, code: &str, state: Option<&str>) -> String {
    let mut url = format!(
        "{redirect_uri}{}code={}",
        if redirect_uri.contains('?') { "&" } else { "?" },
        urlencoding_encode(code)
    );
    if let Some(state) = state {
        if !state.is_empty() {
            url.push_str("&state=");
            url.push_str(&urlencoding_encode(state));
        }
    }
    url
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

async fn login(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    let Some(client) = find_client(&state.config, &form.client_id) else {
        return (StatusCode::BAD_REQUEST, "unknown client_id").into_response();
    };
    if !client.redirect_uris.iter().any(|u| u == &form.redirect_uri) {
        return (StatusCode::BAD_REQUEST, "redirect_uri not registered").into_response();
    }
    if find_user(&state.config, &form.username).is_none() {
        return (StatusCode::BAD_REQUEST, "unknown user").into_response();
    }

    let sid = Uuid::new_v4().to_string();
    let q = AuthorizeQuery {
        response_type: Some("code".into()),
        client_id: Some(form.client_id.clone()),
        redirect_uri: Some(form.redirect_uri.clone()),
        scope: Some(form.scope.clone()),
        state: nonempty(form.state.clone()),
        nonce: nonempty(form.nonce.clone()),
        code_challenge: nonempty(form.code_challenge.clone()),
        code_challenge_method: nonempty(form.code_challenge_method.clone()),
    };

    let code = {
        let mut store = state.inner.lock().await;
        store.sessions.insert(
            sid.clone(),
            Session {
                username: form.username.clone(),
                expires_at: now() + SESSION_TTL_SECS,
            },
        );
        issue_code(
            &mut store,
            &form.username,
            &form.client_id,
            &form.redirect_uri,
            &q,
        )
    };

    let mut response = Redirect::temporary(&build_redirect(
        &form.redirect_uri,
        &code,
        form.state.as_deref(),
    ))
    .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, set_session_cookie(&sid));
    response
}

fn decode_basic_auth(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ")?;
    let raw = STANDARD.decode(encoded).ok()?;
    let text = String::from_utf8(raw).ok()?;
    let (id, secret) = text.split_once(':')?;
    Some((id.to_string(), secret.to_string()))
}

fn verify_pkce(code: &AuthCode, verifier: Option<&str>) -> Result<(), &'static str> {
    let Some(challenge) = code.code_challenge.as_deref() else {
        return Ok(());
    };
    let Some(verifier) = verifier else {
        return Err("code_verifier required");
    };
    let method = code.code_challenge_method.as_deref().unwrap_or("plain");
    match method {
        "S256" => {
            let digest = Sha256::digest(verifier.as_bytes());
            let computed = URL_SAFE_NO_PAD.encode(digest);
            if computed == challenge {
                Ok(())
            } else {
                Err("invalid code_verifier")
            }
        }
        "plain" => {
            if verifier == challenge {
                Ok(())
            } else {
                Err("invalid code_verifier")
            }
        }
        _ => Err("unsupported code_challenge_method"),
    }
}

async fn token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    if form.grant_type.as_deref() != Some("authorization_code") {
        return json_error(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }
    let Some(code_value) = form.code.as_deref() else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_request");
    };

    let (client_id, client_secret) = match (
        form.client_id.clone(),
        form.client_secret.clone(),
        decode_basic_auth(&headers),
    ) {
        (Some(id), secret, _) => (id, secret),
        (_, _, Some((id, secret))) => (id, Some(secret)),
        _ => return json_error(StatusCode::UNAUTHORIZED, "invalid_client"),
    };

    let Some(client) = find_client(&state.config, &client_id) else {
        return json_error(StatusCode::UNAUTHORIZED, "invalid_client");
    };
    // Relaxed: accept missing secret or matching secret.
    if let Some(secret) = client_secret {
        if secret != client.secret && !secret.is_empty() {
            // Still accept mismatched secrets in this deliberately-insecure mode? Plan says
            // "Client-Secret relaxed akzeptieren" — accept any / missing.
            let _ = secret;
        }
    }

    let mut store = state.inner.lock().await;
    let Some(code) = store.codes.remove(code_value) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    if code.expires_at < now() {
        return json_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    if code.client_id != client_id {
        return json_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    if let Some(redirect_uri) = form.redirect_uri.as_deref() {
        if redirect_uri != code.redirect_uri {
            return json_error(StatusCode::BAD_REQUEST, "invalid_grant");
        }
    }
    if let Err(msg) = verify_pkce(&code, form.code_verifier.as_deref()) {
        return json_error(StatusCode::BAD_REQUEST, msg);
    }

    let Some(user) = find_user(&state.config, &code.username).cloned() else {
        return json_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };

    let access_token = Uuid::new_v4().to_string();
    store.access_tokens.insert(
        access_token.clone(),
        TokenRecord {
            username: user.username.clone(),
            client_id: client_id.clone(),
            scope: code.scope.clone(),
            expires_at: now() + TOKEN_TTL_SECS,
        },
    );
    drop(store);

    let iat = now();
    let exp = iat + TOKEN_TTL_SECS;
    let claims = IdTokenClaims {
        iss: state.config.issuer.trim_end_matches('/').to_string(),
        sub: user_sub(&user.username),
        aud: client_id,
        exp,
        iat,
        nonce: code.nonce,
        preferred_username: user.username.clone(),
        email: user.email.clone(),
        email_verified: true,
        extra: user.claims.clone(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(state.kid.clone());
    let id_token = match encode(&header, &claims, &state.encoding_key) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("sign id_token: {e}"),
            )
                .into_response();
        }
    };

    Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": TOKEN_TTL_SECS,
        "id_token": id_token,
        "scope": code.scope,
    }))
    .into_response()
}

fn json_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

async fn userinfo(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    let Some(token) = token else {
        return json_error(StatusCode::UNAUTHORIZED, "invalid_token");
    };
    let store = state.inner.lock().await;
    let Some(record) = store.access_tokens.get(&token).cloned() else {
        return json_error(StatusCode::UNAUTHORIZED, "invalid_token");
    };
    if record.expires_at < now() {
        return json_error(StatusCode::UNAUTHORIZED, "invalid_token");
    }
    drop(store);
    let Some(user) = find_user(&state.config, &record.username) else {
        return json_error(StatusCode::UNAUTHORIZED, "invalid_token");
    };
    let mut body = json!({
        "sub": user_sub(&user.username),
        "preferred_username": user.username,
        "email": user.email,
        "email_verified": true,
    });
    if let Some(obj) = body.as_object_mut() {
        for (k, v) in &user.claims {
            obj.insert(k.clone(), v.clone());
        }
    }
    Json(body).into_response()
}

async fn end_session(
    State(state): State<AppState>,
    Query(q): Query<EndSessionQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(sid) = parse_cookie(&headers, SESSION_COOKIE) {
        let mut store = state.inner.lock().await;
        store.sessions.remove(&sid);
    }

    let redirect = q.post_logout_redirect_uri.as_deref().filter(|u| {
        if !(u.starts_with("http://") || u.starts_with("https://")) {
            return false;
        }
        // Dev mode: accept any http(s) logout redirect.
        let _ = q
            .client_id
            .as_deref()
            .and_then(|id| find_client(&state.config, id));
        true
    });

    let mut response = if let Some(uri) = redirect {
        let mut target = uri.to_string();
        if let Some(state_param) = q.state.as_deref() {
            if !state_param.is_empty() {
                target.push(if target.contains('?') { '&' } else { '?' });
                target.push_str("state=");
                target.push_str(&urlencoding_encode(state_param));
            }
        }
        Redirect::temporary(&target).into_response()
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(
                "<!DOCTYPE html><html><body><h1>Logged out</h1><p>Session cleared.</p></body></html>",
            ))
            .unwrap()
    };
    response
        .headers_mut()
        .insert(header::SET_COOKIE, clear_session_cookie());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration as StdDuration;
    use tokio::task::JoinHandle;

    fn test_config(listen: &str) -> ProviderConfig {
        ProviderConfig {
            issuer: format!("http://{listen}"),
            listen: listen.to_string(),
            clients: vec![ClientConfig {
                id: "web".into(),
                secret: "secret".into(),
                redirect_uris: vec!["http://127.0.0.1:9/oauth2/callback".into()],
            }],
            users: vec![UserConfig {
                username: "alice".into(),
                email: "alice@dev.local".into(),
                claims: HashMap::from([("role".into(), json!("admin"))]),
            }],
        }
    }

    fn http_exchange(addr: &str, request: &str) -> (u16, HashMap<String, String>, String) {
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(StdDuration::from_secs(3)))
            .ok();
        stream.write_all(request.as_bytes()).expect("write");
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        loop {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf);
                    if let Some(pos) = text.find("\r\n\r\n") {
                        if let Some(cl) = text
                            .lines()
                            .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        {
                            let len: usize = cl
                                .split(':')
                                .nth(1)
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0);
                            if buf.len() >= pos + 4 + len {
                                break;
                            }
                        } else if text.starts_with("HTTP/1.1 30") {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status_line = text.lines().next().unwrap_or("");
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut headers = HashMap::new();
        if let Some((head, _)) = text.split_once("\r\n\r\n") {
            for line in head.lines().skip(1) {
                if let Some((k, v)) = line.split_once(':') {
                    headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                }
            }
        }
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        (status, headers, body)
    }

    async fn spawn_server(work: PathBuf, config: ProviderConfig) -> (String, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().unwrap();
        let listen = addr.to_string();
        let mut config = config;
        config.listen = listen.clone();
        config.issuer = format!("http://{listen}");

        let (encoding_key, jwks, kid) = load_or_create_keys(&work).expect("keys");
        let state = AppState {
            inner: Arc::new(Mutex::new(Store {
                sessions: HashMap::new(),
                codes: HashMap::new(),
                access_tokens: HashMap::new(),
            })),
            config: Arc::new(config),
            encoding_key,
            jwks,
            kid,
        };
        let app = Router::new()
            .route("/", get(root))
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks_handler))
            .route("/authorize", get(authorize))
            .route("/login", post(login))
            .route("/token", post(token))
            .route("/userinfo", get(userinfo).post(userinfo))
            .route("/end_session", get(end_session))
            .with_state(state);
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (listen, handle)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn discovery_login_token_logout_flow() {
        let dir = tempfile::tempdir().unwrap();
        let (listen, _handle) =
            spawn_server(dir.path().to_path_buf(), test_config("127.0.0.1:0")).await;

        let (status, _, body) = http_exchange(
            &listen,
            &format!("GET /.well-known/openid-configuration HTTP/1.1\r\nHost: {listen}\r\nConnection: close\r\n\r\n"),
        );
        assert_eq!(status, 200);
        let discovery: Value = serde_json::from_str(&body).unwrap();
        assert!(discovery["end_session_endpoint"]
            .as_str()
            .unwrap()
            .contains("/end_session"));

        let redirect = "http://127.0.0.1:9/oauth2/callback";
        let auth_url = format!(
            "/authorize?response_type=code&client_id=web&redirect_uri={}&scope=openid%20profile%20email&state=xyz&code_challenge=abc&code_challenge_method=plain",
            urlencoding_encode(redirect)
        );
        let (status, _, body) = http_exchange(
            &listen,
            &format!("GET {auth_url} HTTP/1.1\r\nHost: {listen}\r\nConnection: close\r\n\r\n"),
        );
        assert_eq!(status, 200);
        assert!(body.contains("Pick a user"));
        assert!(body.contains("alice"));

        let form = format!(
            "username=alice&client_id=web&redirect_uri={}&scope=openid+profile+email&state=xyz&code_challenge=abc&code_challenge_method=plain",
            urlencoding_encode(redirect)
        );
        let (status, headers, _) = http_exchange(
            &listen,
            &format!(
                "POST /login HTTP/1.1\r\nHost: {listen}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
                form.len()
            ),
        );
        assert!(status == 302 || status == 307, "status={status}");
        let location = headers.get("location").cloned().unwrap_or_default();
        assert!(location.contains("code="), "location={location}");
        let cookie = headers.get("set-cookie").cloned().unwrap_or_default();
        assert!(cookie.contains(SESSION_COOKIE));
        let code = location
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();

        let token_body = format!(
            "grant_type=authorization_code&code={code}&redirect_uri={}&client_id=web&client_secret=secret&code_verifier=abc",
            urlencoding_encode(redirect)
        );
        let (status, _, body) = http_exchange(
            &listen,
            &format!(
                "POST /token HTTP/1.1\r\nHost: {listen}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{token_body}",
                token_body.len()
            ),
        );
        assert_eq!(status, 200, "body={body}");
        let token_json: Value = serde_json::from_str(&body).unwrap();
        assert!(token_json["id_token"].as_str().is_some());
        assert!(token_json["access_token"].as_str().is_some());

        let id_token = token_json["id_token"].as_str().unwrap();
        let payload_b64 = id_token.split('.').nth(1).unwrap();
        let payload_raw = URL_SAFE_NO_PAD.decode(payload_b64).unwrap();
        let payload: Value = serde_json::from_slice(&payload_raw).unwrap();
        assert_eq!(payload["preferred_username"], "alice");
        assert_eq!(payload["email"], "alice@dev.local");
        assert_eq!(payload["role"], "admin");

        let sid = cookie
            .split(';')
            .next()
            .unwrap()
            .strip_prefix(&format!("{SESSION_COOKIE}="))
            .unwrap();
        let (status, headers, _) = http_exchange(
            &listen,
            &format!(
                "GET /end_session?post_logout_redirect_uri={}&client_id=web HTTP/1.1\r\nHost: {listen}\r\nCookie: {SESSION_COOKIE}={sid}\r\nConnection: close\r\n\r\n",
                urlencoding_encode("http://127.0.0.1:9/")
            ),
        );
        assert!(status == 302 || status == 307);
        assert!(headers.get("set-cookie").unwrap().contains("Max-Age=0"));

        let (status, _, body) = http_exchange(
            &listen,
            &format!(
                "GET {auth_url} HTTP/1.1\r\nHost: {listen}\r\nCookie: {SESSION_COOKIE}={sid}\r\nConnection: close\r\n\r\n"
            ),
        );
        assert_eq!(status, 200);
        assert!(body.contains("Pick a user"));
    }
}
