//! Thin async wrappers over the ACP backend REST/SSE API.

mod accounts;
mod auth;
mod errors;
mod paths;
mod request;
mod response;
mod sessions;
mod stream;

pub(crate) use accounts::{create_account, delete_account, list_accounts, update_account};
pub(crate) use auth::{auth_status, bootstrap_register, sign_in, sign_out};
pub(crate) use errors::SessionLoadError;
pub(crate) use paths::{decode_component, encode_component, permission_url, session_path};
pub(crate) use request::{csrf_token, patch_json_with_csrf, post_json_with_csrf};
pub(crate) use response::{classify_session_load_failure, response_error_message};
pub(crate) use sessions::{
    cancel_turn, create_session, delete_session, list_sessions, load_session, rename_session,
    resolve_permission, send_message,
};
pub(crate) use stream::{SseItem, open_session_event_stream};
