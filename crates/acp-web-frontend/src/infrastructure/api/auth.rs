use acp_contracts::{
    AuthStatusResponse, BootstrapRegistrationRequest, BootstrapRegistrationResponse, LocalAccount,
    SignInRequest, SignInResponse,
};
use gloo_net::http::Request;

use super::{csrf_token, post_json_with_csrf, response_error_message};

const AUTH_STATUS_URL: &str = "/api/v1/auth/status";
const BOOTSTRAP_REGISTER_URL: &str = "/api/v1/bootstrap/register";
const SIGN_IN_URL: &str = "/api/v1/auth/sign-in";
const SIGN_OUT_URL: &str = "/api/v1/auth/sign-out";

pub(crate) async fn auth_status() -> Result<AuthStatusResponse, String> {
    let response = Request::get(AUTH_STATUS_URL)
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
    let response = post_json_with_csrf(
        BOOTSTRAP_REGISTER_URL,
        bootstrap_register_body(username, password)?,
    )
    .await?;
    if !response.ok() {
        return Err(response_error_message(response, "Registration failed").await);
    }
    let payload: BootstrapRegistrationResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

pub(crate) async fn sign_in(username: &str, password: &str) -> Result<LocalAccount, String> {
    let response = post_json_with_csrf(SIGN_IN_URL, sign_in_body(username, password)?).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Sign in failed").await);
    }
    let payload: SignInResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

pub(crate) async fn sign_out() -> Result<(), String> {
    let csrf = csrf_token();
    let response = Request::post(SIGN_OUT_URL)
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Sign out failed").await);
    }
    Ok(())
}

fn bootstrap_register_body(username: &str, password: &str) -> Result<String, String> {
    serde_json::to_string(&BootstrapRegistrationRequest {
        username: username.to_string(),
        password: password.to_string(),
    })
    .map_err(|error| error.to_string())
}

fn sign_in_body(username: &str, password: &str) -> Result<String, String> {
    serde_json::to_string(&SignInRequest {
        username: username.to_string(),
        password: password.to_string(),
    })
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_request_bodies_serialize_expected_payloads() {
        assert_eq!(
            bootstrap_register_body("alice", "pw").expect("registration body"),
            r#"{"username":"alice","password":"pw"}"#
        );
        assert_eq!(
            sign_in_body("bob", "secret").expect("sign-in body"),
            r#"{"username":"bob","password":"secret"}"#
        );
    }

    #[test]
    fn auth_endpoints_match_expected_paths() {
        assert_eq!(AUTH_STATUS_URL, "/api/v1/auth/status");
        assert_eq!(BOOTSTRAP_REGISTER_URL, "/api/v1/bootstrap/register");
        assert_eq!(SIGN_IN_URL, "/api/v1/auth/sign-in");
        assert_eq!(SIGN_OUT_URL, "/api/v1/auth/sign-out");
    }
}
