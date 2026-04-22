const PREPARED_SESSION_STORAGE_KEY: &str = "acp-prepared-session-id";
const DRAFT_STORAGE_KEY_PREFIX: &str = "acp-draft-";

pub(crate) fn navigate_to(path: &str) -> Result<(), String> {
    web_sys::window()
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
    web_sys::window().and_then(|window| window.session_storage().ok().flatten())
}

fn draft_storage_key(session_id: &str) -> String {
    format!("{DRAFT_STORAGE_KEY_PREFIX}{session_id}")
}
