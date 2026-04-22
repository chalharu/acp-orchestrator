use super::*;

#[tokio::test]
async fn redirect_to_app_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_app().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/"));
}

#[tokio::test]
async fn redirect_to_register_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_register().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/register/"));
}

#[tokio::test]
async fn redirect_to_sign_in_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_sign_in().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/sign-in/"));
}

#[tokio::test]
async fn redirect_to_accounts_uses_the_canonical_trailing_slash_route() {
    let response = redirect_to_accounts().await.into_response();
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("redirect responses should include a location header");

    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.to_str().ok(), Some("/app/accounts/"));
}

#[tokio::test]
async fn app_shell_csp_permits_wasm_execution() {
    let response = app_entrypoint(HeaderMap::new()).await;
    let csp = response
        .headers()
        .get("content-security-policy")
        .expect("app shell should include a CSP header")
        .to_str()
        .expect("CSP header should be valid UTF-8");

    // WebAssembly execution requires 'wasm-unsafe-eval' in script-src.
    assert!(
        csp.contains("'wasm-unsafe-eval'"),
        "CSP script-src must include 'wasm-unsafe-eval' for WASM; got: {csp}",
    );
}

#[tokio::test]
async fn wasm_init_script_responds_with_javascript_content_type() {
    let response = wasm_init_script().await;
    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("wasm-init.js response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8");

    assert!(ct.starts_with("application/javascript"), "got: {ct}");
}

#[tokio::test]
async fn app_stylesheet_responds_with_css_content_type() {
    let response = app_stylesheet().await;
    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("app.css response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8")
        .to_string();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("app.css body should be readable");
    let body_text = String::from_utf8(body.to_vec()).expect("app.css should be valid UTF-8");

    assert!(ct.starts_with("text/css"), "got: {ct}");
    assert!(!body_text.is_empty());
    assert!(body_text.contains("Noto Sans JP"));
    assert!(body_text.contains(
        ".account-shell {\n  width: min(1160px, 100%);\n  height: 100%;\n  min-height: 0;\n  overflow-y: auto;"
    ));
    assert!(body_text.contains(".account-table-wrap {\n  overflow: auto;"));
}

#[tokio::test]
async fn app_font_asset_responds_with_font_content_type() {
    for font_name in [
        "noto-sans-jp-latin-400.woff2",
        "noto-sans-jp-japanese-400.woff2",
        "noto-sans-jp-latin-500.woff2",
        "noto-sans-jp-japanese-500.woff2",
        "noto-sans-jp-latin-700.woff2",
        "noto-sans-jp-japanese-700.woff2",
    ] {
        let response = app_font_asset(Path(font_name.to_string())).await;
        let ct = response
            .headers()
            .get(CONTENT_TYPE)
            .expect("font asset response should include content-type")
            .to_str()
            .expect("content-type should be valid UTF-8")
            .to_string();
        let cache_control = response
            .headers()
            .get(CACHE_CONTROL)
            .expect("font asset response should include cache-control")
            .to_str()
            .expect("cache-control should be valid UTF-8")
            .to_string();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("font asset body should be readable");

        assert!(ct.starts_with("font/woff2"), "{font_name}: got {ct}");
        assert_eq!(
            cache_control, "public, max-age=31536000, immutable",
            "{font_name}: cache-control mismatch"
        );
        assert!(!body.is_empty(), "{font_name}: body should not be empty");
    }
}

#[tokio::test]
async fn app_font_asset_returns_not_found_for_unknown_names() {
    let response = app_font_asset(Path("missing.ttf".to_string())).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wasm_glue_js_returns_503_when_frontend_dist_is_not_configured() {
    let state = test_state(); // frontend_dist = None
    let response = wasm_glue_javascript(State(state)).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_glue_js_returns_503_when_frontend_js_bundle_is_missing() {
    let response = wasm_glue_javascript(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with(false, true),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_glue_js_returns_503_when_frontend_js_bundle_cannot_be_read() {
    let response = wasm_glue_javascript(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with_unreadable_javascript(),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_glue_js_responds_with_javascript_content_type_when_dist_is_configured() {
    let response = wasm_glue_javascript(State(test_state_with_frontend_dist(
        write_temp_frontend_dist(),
    )))
    .await;

    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("WASM glue JS response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8");

    assert!(ct.starts_with("application/javascript"), "got: {ct}");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn wasm_binary_returns_503_when_frontend_dist_is_not_configured() {
    let state = test_state(); // frontend_dist = None
    let response = wasm_binary(State(state)).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_binary_returns_503_when_frontend_wasm_bundle_is_missing() {
    let response = wasm_binary(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with(true, false),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_binary_returns_503_when_frontend_wasm_bundle_cannot_be_read() {
    let response = wasm_binary(State(test_state_with_frontend_dist(
        write_temp_frontend_dist_with_unreadable_wasm(),
    )))
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn wasm_binary_responds_with_wasm_content_type_when_dist_is_configured() {
    let response = wasm_binary(State(test_state_with_frontend_dist(
        write_temp_frontend_dist(),
    )))
    .await;

    let ct = response
        .headers()
        .get(CONTENT_TYPE)
        .expect("WASM binary response should include content-type")
        .to_str()
        .expect("content-type should be valid UTF-8");

    assert_eq!(ct, "application/wasm", "got: {ct}");
    assert_eq!(response.status(), StatusCode::OK);
}
