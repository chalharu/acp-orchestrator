mod accounts;
mod queries;
mod schema;
mod session_metadata;
mod shared;

const BOOTSTRAP_WORKSPACE_KIND: &str = "legacy-session-routes";
pub(super) const BOOTSTRAP_WORKSPACE_NAME: &str = "Default workspace";
const ACTIVE_WORKSPACE_STATUS: &str = "active";
pub(super) const LOCAL_ACCOUNT_PRINCIPAL_KIND: &str = "local_account";
const LEGACY_BROWSER_SESSIONS_TABLE: &str = "legacy_browser_sessions";

pub(super) use accounts::{
    authenticate_browser_session, authenticate_browser_session_in_transaction,
    authenticate_local_account_in_transaction, bind_browser_session_to_user,
    delete_local_account_in_transaction, insert_local_account, list_local_accounts,
    local_account_count, local_account_from_user, materialize_bearer_user_in_transaction,
    materialize_browser_session_user_in_transaction, soft_delete_browser_session,
    update_local_account_in_transaction,
};
#[cfg(test)]
pub(super) use accounts::{
    durable_local_account_subject, encode_password_salt, hash_password, map_account_write_error,
    next_password_hash, validate_password, validate_username, verify_password,
};
pub(super) use queries::load_session_metadata_record;
pub(super) use schema::initialize_schema;
pub(super) use session_metadata::{
    bootstrap_workspace_in_transaction, build_session_metadata_record, upsert_session_metadata,
};
pub(super) use shared::{
    database_error, ensure_parent_dir, join_error, open_immediate_transaction,
};
#[cfg(test)]
pub(super) use shared::{
    durable_principal_subject, parse_optional_timestamp_for_row, parse_timestamp,
    parse_timestamp_for_row, timestamp,
};
