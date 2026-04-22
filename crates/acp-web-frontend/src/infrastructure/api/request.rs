use gloo_net::http::Request;

/// Read the CSRF token injected by the backend into
/// `<meta name="acp-csrf-token" content="...">`.
pub(crate) fn csrf_token() -> String {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| {
            document
                .query_selector("meta[name='acp-csrf-token']")
                .ok()
                .flatten()
        })
        .and_then(|element| element.get_attribute("content"))
        .unwrap_or_default()
}

pub(crate) async fn post_json_with_csrf(
    url: &str,
    body: String,
) -> Result<gloo_net::http::Response, String> {
    let csrf = csrf_token();
    Request::post(url)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn patch_json_with_csrf(
    url: &str,
    body: String,
) -> Result<gloo_net::http::Response, String> {
    let csrf = csrf_token();
    Request::patch(url)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())
}
