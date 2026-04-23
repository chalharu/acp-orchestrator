#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

#[allow(dead_code)]
const PREPARED_SESSION_STORAGE_KEY: &str = "acp-prepared-session-id";
#[cfg_attr(not(any(test, target_family = "wasm")), allow(dead_code))]
const DRAFT_STORAGE_KEY_PREFIX: &str = "acp-draft-";

#[allow(unused_variables)]
pub(crate) fn navigate_to(path: &str) -> Result<(), String> {
    #[cfg(not(target_family = "wasm"))]
    return Err("window not available".to_string());
    #[cfg(target_family = "wasm")]
    return web_sys::window()
        .ok_or_else(|| "window not available".to_string())?
        .location()
        .set_href(path)
        .map_err(|error| format!("Failed to navigate to {path}: {error:?}"));
}

pub(crate) fn prepared_session_id() -> Option<String> {
    #[cfg(not(target_family = "wasm"))]
    return None;
    #[cfg(target_family = "wasm")]
    return session_storage()
        .and_then(|s| s.get_item(PREPARED_SESSION_STORAGE_KEY).ok().flatten())
        .filter(|id| !id.is_empty());
}

// Parameters below are only referenced inside `#[cfg(target_family = "wasm")]` blocks;
// on host the function bodies are empty, making the params appear unused.
#[allow(unused_variables)]
pub(crate) fn store_prepared_session_id(session_id: &str) {
    #[cfg(target_family = "wasm")]
    if let Some(storage) = session_storage() {
        let _ = storage.set_item(PREPARED_SESSION_STORAGE_KEY, session_id);
    }
}

#[allow(unused_variables)]
pub(crate) fn clear_prepared_session_id_if_matches(session_id: &str) {
    #[cfg(target_family = "wasm")]
    if prepared_session_id().as_deref() == Some(session_id) {
        clear_prepared_session_id();
    }
}

pub(crate) fn clear_prepared_session_id() {
    #[cfg(target_family = "wasm")]
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(PREPARED_SESSION_STORAGE_KEY);
    }
}

#[allow(unused_variables)]
pub(crate) fn load_draft(session_id: &str) -> String {
    #[cfg(not(target_family = "wasm"))]
    return String::new();
    #[cfg(target_family = "wasm")]
    return session_storage()
        .and_then(|s| s.get_item(&draft_storage_key(session_id)).ok().flatten())
        .unwrap_or_default();
}

#[allow(unused_variables)]
pub(crate) fn save_draft(session_id: &str, text: &str) {
    #[cfg(target_family = "wasm")]
    if let Some(storage) = session_storage() {
        if text.is_empty() {
            let _ = storage.remove_item(&draft_storage_key(session_id));
        } else {
            let _ = storage.set_item(&draft_storage_key(session_id), text);
        }
    }
}

#[allow(unused_variables)]
pub(crate) fn clear_draft(session_id: &str) {
    #[cfg(target_family = "wasm")]
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(&draft_storage_key(session_id));
    }
}

#[cfg(target_family = "wasm")]
fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.session_storage().ok().flatten())
}

#[cfg_attr(not(any(test, target_family = "wasm")), allow(dead_code))]
fn draft_storage_key(session_id: &str) -> String {
    format!("{DRAFT_STORAGE_KEY_PREFIX}{session_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_to_returns_an_error_without_a_browser_window() {
        assert!(navigate_to("/app/").is_err());
    }

    #[test]
    fn prepared_session_helpers_fall_back_without_session_storage() {
        assert_eq!(prepared_session_id(), None);
        store_prepared_session_id("session-1");
        clear_prepared_session_id_if_matches("session-1");
        clear_prepared_session_id();
        assert_eq!(prepared_session_id(), None);
    }

    #[test]
    fn draft_helpers_use_empty_defaults_without_session_storage() {
        assert_eq!(load_draft("session-1"), "");
        save_draft("session-1", "draft");
        save_draft("session-1", "");
        clear_draft("session-1");
    }

    #[test]
    fn draft_storage_key_uses_the_expected_prefix() {
        assert_eq!(draft_storage_key("session-1"), "acp-draft-session-1");
    }
}
