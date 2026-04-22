use gloo_net::http::Request;

/// Read the CSRF token injected by the backend into
/// `<meta name="acp-csrf-token" content="...">`.
#[cfg(target_family = "wasm")]
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

#[cfg(not(target_family = "wasm"))]
pub(crate) fn csrf_token() -> String {
    String::new()
}

pub(crate) async fn post_json_with_csrf(
    url: &str,
    body: String,
) -> Result<gloo_net::http::Response, String> {
    post_json_request(url, body, &csrf_token())?
        .send()
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn patch_json_with_csrf(
    url: &str,
    body: String,
) -> Result<gloo_net::http::Response, String> {
    patch_json_request(url, body, &csrf_token())?
        .send()
        .await
        .map_err(|error| error.to_string())
}

fn post_json_request(
    url: &str,
    body: String,
    csrf: &str,
) -> Result<gloo_net::http::Request, String> {
    let [
        (csrf_name, csrf_value),
        (content_type_name, content_type_value),
    ] = json_request_headers(csrf);
    Request::post(url)
        .header(csrf_name, &csrf_value)
        .header(content_type_name, &content_type_value)
        .body(body)
        .map_err(|error| error.to_string())
}

fn patch_json_request(
    url: &str,
    body: String,
    csrf: &str,
) -> Result<gloo_net::http::Request, String> {
    let [
        (csrf_name, csrf_value),
        (content_type_name, content_type_value),
    ] = json_request_headers(csrf);
    Request::patch(url)
        .header(csrf_name, &csrf_value)
        .header(content_type_name, &content_type_value)
        .body(body)
        .map_err(|error| error.to_string())
}

fn json_request_headers(csrf: &str) -> [(&'static str, String); 2] {
    [
        ("x-csrf-token", csrf.to_string()),
        ("content-type", "application/json".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_token_defaults_to_empty_string_without_browser_meta_tag() {
        assert!(csrf_token().is_empty());
    }

    #[test]
    fn json_request_headers_include_csrf_and_json_content_type() {
        assert_eq!(
            json_request_headers("csrf-token"),
            [
                ("x-csrf-token", "csrf-token".to_string()),
                ("content-type", "application/json".to_string()),
            ]
        );
    }
}
