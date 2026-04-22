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
    delete_local_account_in_transaction, durable_local_account_subject, encode_password_salt,
    hash_password, insert_local_account, list_local_accounts, local_account_count,
    local_account_from_user, map_account_write_error, materialize_bearer_user_in_transaction,
    materialize_browser_session_user_in_transaction, next_password_hash,
    soft_delete_browser_session, update_local_account_in_transaction, validate_password,
    validate_username, verify_password,
};
pub(super) use queries::{
    load_active_local_account_by_username, load_session_metadata_record, load_user_by_id,
    load_user_by_principal,
};
pub(super) use schema::initialize_schema;
pub(super) use session_metadata::{
    bootstrap_workspace_in_transaction, build_session_metadata_record, upsert_session_metadata,
};
pub(super) use shared::{
    database_error, durable_principal_subject, ensure_parent_dir, join_error,
    open_immediate_transaction, parse_optional_timestamp_for_row, parse_timestamp,
    parse_timestamp_for_row, timestamp,
};
