#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use crate::{
    infrastructure::api,
    routing::{AppRoute, route_from_pathname},
};

pub(super) fn path_with_return_to(base_path: &str, return_to_path: &str) -> String {
    format!(
        "{base_path}?return_to={}",
        api::encode_component(return_to_path)
    )
}

#[cfg(target_family = "wasm")]
pub(super) fn session_return_to_path_from_location() -> Option<String> {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| session_return_to_path(&search))
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn session_return_to_path_from_location() -> Option<String> {
    None
}

pub(super) fn session_return_to_path(search: &str) -> Option<String> {
    query_param(search, "return_to")
        .filter(|path| matches!(route_from_pathname(path), AppRoute::Session(_)))
}

fn query_param(search: &str, name: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name)
                .then(|| api::decode_component(value))
                .flatten()
        })
}

#[cfg(test)]
mod tests {
    use super::{path_with_return_to, query_param, session_return_to_path};

    #[test]
    fn return_to_paths_round_trip_only_session_routes() {
        assert_eq!(
            path_with_return_to("/app/workspaces/", "/app/sessions/s%2F1"),
            "/app/workspaces/?return_to=%2Fapp%2Fsessions%2Fs%252F1"
        );
        assert_eq!(
            session_return_to_path("?return_to=%2Fapp%2Fsessions%2Fs%252F1"),
            Some("/app/sessions/s%2F1".to_string())
        );
        assert_eq!(session_return_to_path("?return_to=%2Fapp%2F"), None);
        assert_eq!(
            session_return_to_path("?return_to=https%3A%2F%2Fexample.com"),
            None
        );
    }

    #[test]
    fn query_param_returns_named_values() {
        assert_eq!(
            query_param("?return_to=%2Fapp%2Fsessions%2Fabc&x=1", "return_to"),
            Some("/app/sessions/abc".to_string())
        );
        assert_eq!(query_param("?x=1", "return_to"), None);
        assert_eq!(
            query_param("return_to=%2Fapp%2F", "return_to"),
            Some("/app/".to_string())
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn session_return_to_path_from_location_returns_none_without_browser() {
        assert_eq!(super::session_return_to_path_from_location(), None);
    }
}
