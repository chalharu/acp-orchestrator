#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts::{
    AccountListResponse, CreateAccountRequest, DeleteAccountResponse, LocalAccount,
    UpdateAccountRequest,
};
#[cfg(target_family = "wasm")]
use acp_contracts::{CreateAccountResponse, UpdateAccountResponse};
#[cfg(target_family = "wasm")]
use gloo_net::http::Request;

#[cfg(target_family = "wasm")]
use super::response_error_message;
#[cfg(target_family = "wasm")]
use super::{csrf_token, patch_json_with_csrf, post_json_with_csrf};

const ACCOUNTS_URL: &str = "/api/v1/accounts";

#[cfg(target_family = "wasm")]
pub(crate) async fn list_accounts() -> Result<AccountListResponse, String> {
    let response = Request::get(ACCOUNTS_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Load accounts failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_accounts() -> Result<AccountListResponse, String> {
    Err(non_wasm_api_error("GET", ACCOUNTS_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn create_account(
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<LocalAccount, String> {
    let response = post_json_with_csrf(
        ACCOUNTS_URL,
        create_account_body(username, password, is_admin)?,
    )
    .await?;
    if !response.ok() {
        return Err(response_error_message(response, "Create account failed").await);
    }
    let payload: CreateAccountResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_account(
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<LocalAccount, String> {
    let _ = create_account_body(username, password, is_admin)?;
    Err(non_wasm_api_error("POST", ACCOUNTS_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn update_account(
    user_id: &str,
    password: Option<String>,
    is_admin: Option<bool>,
) -> Result<LocalAccount, String> {
    let response = patch_json_with_csrf(
        &account_url(user_id),
        update_account_body(password, is_admin)?,
    )
    .await?;
    if !response.ok() {
        return Err(response_error_message(response, "Update account failed").await);
    }
    let payload: UpdateAccountResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn update_account(
    user_id: &str,
    password: Option<String>,
    is_admin: Option<bool>,
) -> Result<LocalAccount, String> {
    let _ = update_account_body(password, is_admin)?;
    Err(non_wasm_api_error("PATCH", &account_url(user_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn delete_account(user_id: &str) -> Result<DeleteAccountResponse, String> {
    let csrf = csrf_token();
    let response = Request::delete(&account_url(user_id))
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Delete account failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn delete_account(user_id: &str) -> Result<DeleteAccountResponse, String> {
    Err(non_wasm_api_error("DELETE", &account_url(user_id)))
}

fn create_account_body(username: &str, password: &str, is_admin: bool) -> Result<String, String> {
    serde_json::to_string(&CreateAccountRequest {
        username: username.to_string(),
        password: password.to_string(),
        is_admin,
    })
    .map_err(|error| error.to_string())
}

fn update_account_body(password: Option<String>, is_admin: Option<bool>) -> Result<String, String> {
    serde_json::to_string(&UpdateAccountRequest { password, is_admin })
        .map_err(|error| error.to_string())
}

fn account_url(user_id: &str) -> String {
    format!("{ACCOUNTS_URL}/{user_id}")
}

fn non_wasm_api_error(method: &str, url: &str) -> String {
    format!("Browser {method} accounts API is unavailable on non-wasm targets: {url}")
}

#[cfg(test)]
mod tests {
    use crate::infrastructure::api::poll_ready;

    use super::*;

    #[test]
    fn account_request_bodies_serialize_expected_payloads() {
        assert_eq!(
            create_account_body("alice", "pw", true).expect("create body"),
            r#"{"username":"alice","password":"pw","is_admin":true}"#
        );
        assert_eq!(
            update_account_body(Some("pw".to_string()), Some(false)).expect("update body"),
            r#"{"password":"pw","is_admin":false}"#
        );
    }

    #[test]
    fn account_url_appends_the_user_id() {
        assert_eq!(account_url("user-1"), "/api/v1/accounts/user-1");
        assert_eq!(ACCOUNTS_URL, "/api/v1/accounts");
    }

    #[test]
    fn host_accounts_api_functions_fail_with_descriptive_errors() {
        let list_error = poll_ready(list_accounts()).expect_err("host list should fail");
        assert!(list_error.contains(ACCOUNTS_URL));

        let create_error =
            poll_ready(create_account("alice", "pw", true)).expect_err("host create should fail");
        assert!(create_error.contains(ACCOUNTS_URL));

        let update_error = poll_ready(update_account(
            "user-1",
            Some("pw".to_string()),
            Some(false),
        ))
        .expect_err("host update should fail");
        assert!(update_error.contains("/api/v1/accounts/user-1"));

        let delete_error =
            poll_ready(delete_account("user-1")).expect_err("host delete should fail");
        assert!(delete_error.contains("/api/v1/accounts/user-1"));
    }
}
