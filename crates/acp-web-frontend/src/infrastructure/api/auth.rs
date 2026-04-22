use acp_contracts::{
    AuthStatusResponse, BootstrapRegistrationRequest, BootstrapRegistrationResponse, LocalAccount,
    SignInRequest, SignInResponse,
};
use gloo_net::http::Request;

use super::{csrf_token, post_json_with_csrf, response_error_message};

pub(crate) async fn auth_status() -> Result<AuthStatusResponse, String> {
    let response = Request::get("/api/v1/auth/status")
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Load auth status failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

pub(crate) async fn bootstrap_register(
    username: &str,
    password: &str,
) -> Result<LocalAccount, String> {
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

pub(crate) async fn sign_in(username: &str, password: &str) -> Result<LocalAccount, String> {
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

pub(crate) async fn sign_out() -> Result<(), String> {
    let csrf = csrf_token();
    let response = Request::post("/api/v1/auth/sign-out")
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Sign out failed").await);
    }
    Ok(())
}
