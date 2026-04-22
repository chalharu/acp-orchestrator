use std::{
    io,
    path::{Path as FsPath, PathBuf},
};

use axum::{
    Router,
    extract::{Path, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, REFERRER_POLICY, SET_COOKIE},
    },
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    contract_health::HealthResponse,
    support::frontend::{
        FRONTEND_JAVASCRIPT_ASSET_PATH, FRONTEND_WASM_ASSET_PATH, FrontendBundleAsset,
        LEGACY_FRONTEND_JAVASCRIPT_ASSET_PATH, LEGACY_FRONTEND_WASM_ASSET_PATH,
        find_frontend_bundle_asset,
    },
};

use super::{AppState, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME, cookie_value};

pub(super) fn install_frontend_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/healthz", get(healthz))
        .route("/app", get(redirect_to_app))
        .route("/app/", get(app_entrypoint))
        .route("/app/register", get(redirect_to_register))
        .route("/app/register/", get(app_register_entrypoint))
        .route("/app/sign-in", get(redirect_to_sign_in))
        .route("/app/sign-in/", get(app_sign_in_entrypoint))
        .route("/app/accounts", get(redirect_to_accounts))
        .route("/app/accounts/", get(app_accounts_entrypoint))
        .route("/app/assets/app.css", get(app_stylesheet))
        .route("/app/assets/fonts/{font_name}", get(app_font_asset))
        .route("/app/assets/wasm-init.js", get(wasm_init_script))
        .route(FRONTEND_JAVASCRIPT_ASSET_PATH, get(wasm_glue_javascript))
        .route(FRONTEND_WASM_ASSET_PATH, get(wasm_binary))
        .route(
            LEGACY_FRONTEND_JAVASCRIPT_ASSET_PATH,
            get(wasm_glue_javascript),
        )
        .route(LEGACY_FRONTEND_WASM_ASSET_PATH, get(wasm_binary))
        .route("/app/sessions/{session_id}", get(app_session_entrypoint))
}

pub(super) async fn healthz() -> axum::Json<HealthResponse> {
    axum::Json(HealthResponse {
        status: "ok".to_string(),
    })
}

pub(super) async fn redirect_to_app() -> Redirect {
    Redirect::permanent("/app/")
}

pub(super) async fn redirect_to_register() -> Redirect {
    Redirect::permanent("/app/register/")
}

pub(super) async fn redirect_to_sign_in() -> Redirect {
    Redirect::permanent("/app/sign-in/")
}

pub(super) async fn redirect_to_accounts() -> Redirect {
    Redirect::permanent("/app/accounts/")
}

pub(super) async fn app_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

pub(super) async fn app_register_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

pub(super) async fn app_sign_in_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

pub(super) async fn app_accounts_entrypoint(headers: HeaderMap) -> Response {
    app_shell_response(&headers)
}

pub(super) async fn app_session_entrypoint(
    Path(_session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    app_shell_response(&headers)
}

pub(super) async fn app_stylesheet() -> Response {
    app_static_text_response("text/css; charset=utf-8", APP_STYLESHEET)
}

pub(super) async fn app_font_asset(Path(font_name): Path<String>) -> Response {
    match font_name.as_str() {
        "noto-sans-jp-latin-400.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_LATIN_REGULAR)
        }
        "noto-sans-jp-japanese-400.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_JAPANESE_REGULAR)
        }
        "noto-sans-jp-latin-500.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_LATIN_MEDIUM)
        }
        "noto-sans-jp-japanese-500.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_JAPANESE_MEDIUM)
        }
        "noto-sans-jp-latin-700.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_LATIN_BOLD)
        }
        "noto-sans-jp-japanese-700.woff2" => {
            app_static_font_response(APP_FONT_NOTO_SANS_JP_JAPANESE_BOLD)
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(super) async fn wasm_init_script() -> Response {
    app_static_text_response("application/javascript; charset=utf-8", WASM_INIT_JS)
}

pub(super) async fn wasm_glue_javascript(State(state): State<AppState>) -> Response {
    let asset_path = match locate_frontend_asset(
        &state,
        FrontendBundleAsset::JavaScript,
        "wasm_glue_javascript",
    ) {
        Ok(path) => path,
        Err(detail) => return frontend_unavailable_response_detail(&detail),
    };

    match tokio::fs::read_to_string(&asset_path).await {
        Ok(content) => app_dynamic_text_response("application/javascript; charset=utf-8", content),
        Err(err) => {
            tracing::warn!(%err, path = %asset_path.display(), "failed to read frontend javascript bundle");
            frontend_unavailable_response("wasm_glue_javascript: file not found")
        }
    }
}

pub(super) async fn wasm_binary(State(state): State<AppState>) -> Response {
    let asset_path = match locate_frontend_asset(&state, FrontendBundleAsset::Wasm, "wasm_binary") {
        Ok(path) => path,
        Err(detail) => return frontend_unavailable_response_detail(&detail),
    };

    match tokio::fs::read(&asset_path).await {
        Ok(bytes) => {
            let headers = asset_response_headers("application/wasm");
            (headers, bytes).into_response()
        }
        Err(err) => {
            tracing::warn!(%err, path = %asset_path.display(), "failed to read frontend wasm bundle");
            frontend_unavailable_response("wasm_binary: file not found")
        }
    }
}

fn locate_frontend_asset(
    state: &AppState,
    asset_type: FrontendBundleAsset,
    context_name: &'static str,
) -> Result<PathBuf, String> {
    let Some(dist) = state.frontend_dist.as_deref() else {
        return Err(format!("{context_name}: frontend_dist not configured"));
    };

    let locate_result = match asset_type {
        FrontendBundleAsset::JavaScript => frontend_javascript_asset_path(dist),
        FrontendBundleAsset::Wasm => frontend_wasm_asset_path(dist),
    };

    match locate_result {
        Ok(path) => Ok(path),
        Err(err) => {
            tracing::warn!(%err, asset = ?asset_type, context_name, "failed to locate frontend bundle asset");
            Err(format!("{context_name}: file not found"))
        }
    }
}

fn frontend_javascript_asset_path(dist: &FsPath) -> io::Result<PathBuf> {
    find_frontend_bundle_asset(dist, FrontendBundleAsset::JavaScript)
}

fn frontend_wasm_asset_path(dist: &FsPath) -> io::Result<PathBuf> {
    find_frontend_bundle_asset(dist, FrontendBundleAsset::Wasm)
}

fn frontend_unavailable_response(detail: &'static str) -> Response {
    frontend_unavailable_response_detail(detail)
}

fn frontend_unavailable_response_detail(detail: &str) -> Response {
    tracing::debug!(detail, "frontend WASM assets not available");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Web frontend assets not available. Run `cargo run -- --web` to build and serve them.",
    )
        .into_response()
}

fn app_shell_response(headers: &HeaderMap) -> Response {
    let (existing_session_id, session_id) = app_shell_cookie(headers, SESSION_COOKIE_NAME);
    let (existing_csrf_token, csrf_token) = app_shell_cookie(headers, CSRF_COOKIE_NAME);

    (
        build_app_shell_headers(
            existing_session_id.as_deref(),
            &session_id,
            existing_csrf_token.as_deref(),
            &csrf_token,
        ),
        app_shell_document(&csrf_token),
    )
        .into_response()
}

fn app_static_text_response(content_type: &'static str, body: &'static str) -> Response {
    let response_headers = asset_response_headers(content_type);
    (response_headers, body).into_response()
}

fn app_static_font_response(body: &'static [u8]) -> Response {
    let response_headers =
        asset_response_headers_with_cache("font/woff2", "public, max-age=31536000, immutable");
    (response_headers, body).into_response()
}

fn app_dynamic_text_response(content_type: &'static str, body: String) -> Response {
    let response_headers = asset_response_headers(content_type);
    (response_headers, body).into_response()
}

fn asset_response_headers(content_type: &'static str) -> HeaderMap {
    asset_response_headers_with_cache(content_type, "no-store")
}

fn asset_response_headers_with_cache(
    content_type: &'static str,
    cache_control: &'static str,
) -> HeaderMap {
    let mut response_headers = HeaderMap::new();
    response_headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    response_headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response_headers
}

fn app_shell_cookie(headers: &HeaderMap, name: &str) -> (Option<String>, String) {
    let existing = cookie_uuid_value(headers, name);
    let value = existing
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    (existing, value)
}

fn build_app_shell_headers(
    existing_session_id: Option<&str>,
    session_id: &str,
    existing_csrf_token: Option<&str>,
    csrf_token: &str,
) -> HeaderMap {
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response_headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; style-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'",
        ),
    );
    response_headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    append_cookie_if_missing(
        &mut response_headers,
        existing_session_id,
        SESSION_COOKIE_NAME,
        session_id,
        true,
    );
    append_cookie_if_missing(
        &mut response_headers,
        existing_csrf_token,
        CSRF_COOKIE_NAME,
        csrf_token,
        false,
    );
    response_headers
}

pub(super) fn sign_out_response_headers() -> HeaderMap {
    let mut response_headers = HeaderMap::new();
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response_headers.append(
        SET_COOKIE,
        build_expired_cookie_header(SESSION_COOKIE_NAME, true),
    );
    response_headers.append(
        SET_COOKIE,
        build_expired_cookie_header(CSRF_COOKIE_NAME, false),
    );
    response_headers
}

fn append_cookie_if_missing(
    headers: &mut HeaderMap,
    existing: Option<&str>,
    name: &str,
    value: &str,
    http_only: bool,
) {
    if existing.is_none() {
        headers.append(SET_COOKIE, build_cookie_header(name, value, http_only));
    }
}

const APP_SHELL_DOCUMENT_TEMPLATE: &str = include_str!("../app_assets/app.html");
const APP_STYLESHEET: &str = include_str!("../app_assets/app.css");
const APP_FONT_NOTO_SANS_JP_LATIN_REGULAR: &[u8] =
    include_bytes!("../app_assets/fonts/noto-sans-jp-latin-400-normal.woff2");
const APP_FONT_NOTO_SANS_JP_JAPANESE_REGULAR: &[u8] =
    include_bytes!("../app_assets/fonts/noto-sans-jp-japanese-400-normal.woff2");
const APP_FONT_NOTO_SANS_JP_LATIN_MEDIUM: &[u8] =
    include_bytes!("../app_assets/fonts/noto-sans-jp-latin-500-normal.woff2");
const APP_FONT_NOTO_SANS_JP_JAPANESE_MEDIUM: &[u8] =
    include_bytes!("../app_assets/fonts/noto-sans-jp-japanese-500-normal.woff2");
const APP_FONT_NOTO_SANS_JP_LATIN_BOLD: &[u8] =
    include_bytes!("../app_assets/fonts/noto-sans-jp-latin-700-normal.woff2");
const APP_FONT_NOTO_SANS_JP_JAPANESE_BOLD: &[u8] =
    include_bytes!("../app_assets/fonts/noto-sans-jp-japanese-700-normal.woff2");
const WASM_INIT_JS: &str = "import init from \"./acp-web-frontend.js\";\n\nawait init();\n";

fn app_shell_document(csrf_token: &str) -> Html<String> {
    assert!(
        APP_SHELL_DOCUMENT_TEMPLATE.contains("__ACP_CSRF_TOKEN__"),
        "app.html must contain the __ACP_CSRF_TOKEN__ sentinel",
    );
    Html(APP_SHELL_DOCUMENT_TEMPLATE.replace("__ACP_CSRF_TOKEN__", csrf_token))
}

fn build_cookie_header(name: &str, value: &str, http_only: bool) -> HeaderValue {
    let http_only = if http_only { "; HttpOnly" } else { "" };
    assert!(
        cookie_name_is_safe(name),
        "web app cookie names must stay header-safe",
    );
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "web app cookie values must stay UUID-safe",
    );
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; SameSite=Strict; Secure{http_only}"
    ))
    .expect("web app cookies should serialize into response headers")
}

fn build_expired_cookie_header(name: &str, http_only: bool) -> HeaderValue {
    let http_only = if http_only { "; HttpOnly" } else { "" };
    assert!(
        cookie_name_is_safe(name),
        "web app cookie names must stay header-safe",
    );
    HeaderValue::from_str(&format!(
        "{name}=deleted; Path=/; Max-Age=0; SameSite=Strict; Secure{http_only}"
    ))
    .expect("expired web app cookies should serialize into response headers")
}

fn cookie_name_is_safe(name: &str) -> bool {
    name.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn cookie_uuid_value(headers: &HeaderMap, name: &str) -> Option<String> {
    cookie_value(headers, name).and_then(|value| {
        Uuid::parse_str(value)
            .ok()
            .map(|uuid| uuid.as_hyphenated().to_string())
    })
}

pub(super) fn current_browser_session_id(headers: &HeaderMap) -> Option<String> {
    cookie_uuid_value(headers, SESSION_COOKIE_NAME)
}

#[derive(Debug, Deserialize)]
pub(super) struct SlashCompletionsQuery {
    #[serde(rename = "sessionId")]
    pub(super) session_id: String,
    #[serde(default)]
    pub(super) prefix: String,
}
