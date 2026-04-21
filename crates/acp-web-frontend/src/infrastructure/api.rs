use acp_contracts::{
    AccountListResponse, AuthStatusResponse, BootstrapRegistrationRequest,
    BootstrapRegistrationResponse, CreateAccountRequest, CreateAccountResponse,
    DeleteAccountResponse, ErrorResponse, LocalAccount, SignInRequest, SignInResponse,
    UpdateAccountRequest, UpdateAccountResponse,
};
use gloo_net::http::Request;

pub async fn auth_status() -> Result<AuthStatusResponse, String> {
    let response = Request::get("/api/v1/auth/status")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Load auth status failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

pub async fn bootstrap_register(username: &str, password: &str) -> Result<LocalAccount, String> {
    let body = serde_json::to_string(&BootstrapRegistrationRequest {
        username: username.to_string(),
        password: password.to_string(),
    })
    .map_err(|error| error.to_string())?;
    let response = post_json_with_csrf("/api/v1/bootstrap/register", body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Registration failed").await);
    }
    let payload: BootstrapRegistrationResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

pub async fn sign_in(username: &str, password: &str) -> Result<LocalAccount, String> {
    let body = serde_json::to_string(&SignInRequest {
        username: username.to_string(),
        password: password.to_string(),
    })
    .map_err(|error| error.to_string())?;
    let response = post_json_with_csrf("/api/v1/auth/sign-in", body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Sign in failed").await);
    }
    let payload: SignInResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

pub async fn list_accounts() -> Result<AccountListResponse, String> {
    let response = Request::get("/api/v1/accounts")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Load accounts failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

pub async fn create_account(
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

pub async fn update_account(
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

pub async fn delete_account(user_id: &str) -> Result<DeleteAccountResponse, String> {
    let csrf = crate::api::csrf_token();
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

async fn post_json_with_csrf(url: &str, body: String) -> Result<gloo_net::http::Response, String> {
    let csrf = crate::api::csrf_token();
    Request::post(url)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())
}

async fn patch_json_with_csrf(url: &str, body: String) -> Result<gloo_net::http::Response, String> {
    let csrf = crate::api::csrf_token();
    Request::patch(url)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())
}

async fn response_error_message(response: gloo_net::http::Response, fallback: &str) -> String {
    let status = response.status();
    match response.json::<ErrorResponse>().await {
        Ok(error) if !error.error.trim().is_empty() => error.error,
        _ => format!("{fallback} (status {status})"),
    }
}
