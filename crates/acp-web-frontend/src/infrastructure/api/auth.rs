#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_accounts::{
    AuthStatusResponse, BootstrapRegistrationRequest, LocalAccount, SignInRequest,
};
#[cfg(target_family = "wasm")]
use acp_contracts_accounts::{BootstrapRegistrationResponse, SignInResponse};
#[cfg(target_family = "wasm")]
use gloo_net::http::Request;

#[cfg(target_family = "wasm")]
use super::response_error_message;
#[cfg(target_family = "wasm")]
use super::{csrf_token, post_json_with_csrf};

const AUTH_STATUS_URL: &str = "/api/v1/auth/status";
const BOOTSTRAP_REGISTER_URL: &str = "/api/v1/bootstrap/register";
const SIGN_IN_URL: &str = "/api/v1/auth/sign-in";
const SIGN_OUT_URL: &str = "/api/v1/auth/sign-out";

#[cfg(target_family = "wasm")]
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

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn auth_status() -> Result<AuthStatusResponse, String> {
    Err(non_wasm_api_error("GET", AUTH_STATUS_URL))
}

#[cfg(target_family = "wasm")]
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

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn bootstrap_register(
    username: &str,
    password: &str,
) -> Result<LocalAccount, String> {
    let _ = bootstrap_register_body(username, password)?;
    Err(non_wasm_api_error("POST", BOOTSTRAP_REGISTER_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn sign_in(username: &str, password: &str) -> Result<LocalAccount, String> {
    let response = post_json_with_csrf(SIGN_IN_URL, sign_in_body(username, password)?).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Sign in failed").await);
    }
    let payload: SignInResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.account)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn sign_in(username: &str, password: &str) -> Result<LocalAccount, String> {
    let _ = sign_in_body(username, password)?;
    Err(non_wasm_api_error("POST", SIGN_IN_URL))
}

#[cfg(target_family = "wasm")]
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

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn sign_out() -> Result<(), String> {
    Err(non_wasm_api_error("POST", SIGN_OUT_URL))
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

fn non_wasm_api_error(method: &str, url: &str) -> String {
    format!("Browser {method} auth API is unavailable on non-wasm targets: {url}")
}

#[cfg(test)]
mod tests {
    use crate::infrastructure::api::poll_ready;

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

    #[test]
    fn host_auth_api_functions_fail_with_descriptive_errors() {
        let auth_status_error =
            poll_ready(auth_status()).expect_err("host auth status should fail");
        assert!(auth_status_error.contains(AUTH_STATUS_URL));

        let register_error =
            poll_ready(bootstrap_register("alice", "pw")).expect_err("host register should fail");
        assert!(register_error.contains(BOOTSTRAP_REGISTER_URL));

        let sign_in_error =
            poll_ready(sign_in("alice", "pw")).expect_err("host sign in should fail");
        assert!(sign_in_error.contains(SIGN_IN_URL));

        let sign_out_error = poll_ready(sign_out()).expect_err("host sign out should fail");
        assert!(sign_out_error.contains(SIGN_OUT_URL));
    }
}
