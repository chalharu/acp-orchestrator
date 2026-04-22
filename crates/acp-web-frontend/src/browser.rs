const PREPARED_SESSION_STORAGE_KEY: &str = "acp-prepared-session-id";
const DRAFT_STORAGE_KEY_PREFIX: &str = "acp-draft-";

pub(crate) fn navigate_to(path: &str) -> Result<(), String> {
    browser_window()
        .ok_or_else(|| "window not available".to_string())?
        .location()
        .set_href(path)
        .map_err(|error| format!("Failed to navigate to {path}: {error:?}"))
}

pub(crate) fn prepared_session_id() -> Option<String> {
    session_storage()
        .and_then(|storage| {
            storage
                .get_item(PREPARED_SESSION_STORAGE_KEY)
                .ok()
                .flatten()
        })
        .filter(|session_id| !session_id.is_empty())
}

pub(crate) fn store_prepared_session_id(session_id: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.set_item(PREPARED_SESSION_STORAGE_KEY, session_id);
    }
}

pub(crate) fn clear_prepared_session_id_if_matches(session_id: &str) {
    if prepared_session_id().as_deref() == Some(session_id) {
        clear_prepared_session_id();
    }
}

pub(crate) fn clear_prepared_session_id() {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(PREPARED_SESSION_STORAGE_KEY);
    }
}

pub(crate) fn load_draft(session_id: &str) -> String {
    session_storage()
        .and_then(|storage| {
            storage
                .get_item(&draft_storage_key(session_id))
                .ok()
                .flatten()
        })
        .unwrap_or_default()
}

pub(crate) fn save_draft(session_id: &str, text: &str) {
    if let Some(storage) = session_storage() {
        if text.is_empty() {
            let _ = storage.remove_item(&draft_storage_key(session_id));
        } else {
            let _ = storage.set_item(&draft_storage_key(session_id), text);
        }
    }
}

pub(crate) fn clear_draft(session_id: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(&draft_storage_key(session_id));
    }
}

fn session_storage() -> Option<web_sys::Storage> {
    browser_window().and_then(|window| window.session_storage().ok().flatten())
}

fn browser_window() -> Option<web_sys::Window> {
    #[cfg(target_family = "wasm")]
    {
        web_sys::window()
    }
    #[cfg(not(target_family = "wasm"))]
    {
        None
    }
}

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
