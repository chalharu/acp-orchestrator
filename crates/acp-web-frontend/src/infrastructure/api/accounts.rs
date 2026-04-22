use acp_contracts::{
    AccountListResponse, CreateAccountRequest, CreateAccountResponse, DeleteAccountResponse,
    LocalAccount, UpdateAccountRequest, UpdateAccountResponse,
};
use gloo_net::http::Request;

use super::{csrf_token, patch_json_with_csrf, post_json_with_csrf, response_error_message};

const ACCOUNTS_URL: &str = "/api/v1/accounts";

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

#[cfg(test)]
mod tests {
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
}
