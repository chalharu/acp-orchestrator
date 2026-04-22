use acp_contracts::{
    AccountListResponse, CreateAccountRequest, CreateAccountResponse, DeleteAccountResponse,
    LocalAccount, UpdateAccountRequest, UpdateAccountResponse,
};
use gloo_net::http::Request;

use super::{csrf_token, patch_json_with_csrf, post_json_with_csrf, response_error_message};

pub(crate) async fn list_accounts() -> Result<AccountListResponse, String> {
    let response = Request::get("/api/v1/accounts")
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
    let body = serde_json::to_string(&CreateAccountRequest {
        username: username.to_string(),
        password: password.to_string(),
        is_admin,
    })
    .map_err(|error| error.to_string())?;
    let response = post_json_with_csrf("/api/v1/accounts", body).await?;
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
    let body = serde_json::to_string(&UpdateAccountRequest { password, is_admin })
        .map_err(|error| error.to_string())?;
    let response = patch_json_with_csrf(&format!("/api/v1/accounts/{user_id}"), body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Update account failed").await);
    }
    let payload: UpdateAccountResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

pub(crate) async fn delete_account(user_id: &str) -> Result<DeleteAccountResponse, String> {
    let csrf = csrf_token();
    let response = Request::delete(&format!("/api/v1/accounts/{user_id}"))
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Delete account failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}
