use axum::http::{
    HeaderMap,
    header::{AUTHORIZATION, COOKIE},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

pub const SESSION_COOKIE_NAME: &str = "acp_session";
pub const CSRF_COOKIE_NAME: &str = "acp_csrf";
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub id: String,
    pub kind: AuthenticatedPrincipalKind,
    pub subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatedPrincipalKind {
    Bearer,
    BrowserSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    MissingAuthentication,
    InvalidAuthentication,
    MissingCsrfToken,
    InvalidCsrfToken,
}

impl AuthError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::MissingAuthentication => "missing authentication",
            Self::InvalidAuthentication => "invalid authentication",
            Self::MissingCsrfToken => "missing csrf token",
            Self::InvalidCsrfToken => "invalid csrf token",
        }
    }
}

/// Authorize loopback requests.
///
/// `requires_csrf` applies only to cookie-authenticated browser requests. Bearer-authenticated
/// loopback clients rely on the `Authorization` header and intentionally bypass CSRF.
/// This helper is header-only: it cannot resolve browser cookies into durable signed-in users.
/// Production web routing must use the stateful resolver in `server.rs` instead of treating the
/// raw `acp_session` UUID as the browser principal.
pub fn authorize_request(
    headers: &HeaderMap,
    requires_csrf: bool,
) -> Result<AuthenticatedPrincipal, AuthError> {
    if let Some(principal) = bearer_principal(headers)? {
        return Ok(principal);
    }

    let principal = browser_principal_from_token(browser_session_token(headers)?);
    if requires_csrf {
        validate_csrf(headers)?;
    }
    Ok(principal)
}

pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .filter_map(|entry| {
            let (cookie_name, value) = entry.trim().split_once('=')?;
            (cookie_name == name).then_some(value.trim())
        })
        .find(|value| !value.is_empty())
}

pub fn bearer_principal(headers: &HeaderMap) -> Result<Option<AuthenticatedPrincipal>, AuthError> {
    // Slice 0 bearer identity is loopback-only trust, not secret-based authentication.
    // Any future non-loopback exposure must replace this with real token validation.
    let Some(value) = headers.get(AUTHORIZATION) else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|_| AuthError::InvalidAuthentication)?;
    let token = raw
        .trim_start()
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidAuthentication)?
        .trim();
    if token.is_empty() {
        return Err(AuthError::InvalidAuthentication);
    }

    Ok(Some(AuthenticatedPrincipal {
        id: token.to_string(),
        kind: AuthenticatedPrincipalKind::Bearer,
        subject: token.to_string(),
    }))
}

pub fn browser_session_token(headers: &HeaderMap) -> Result<String, AuthError> {
    let token =
        cookie_value(headers, SESSION_COOKIE_NAME).ok_or(AuthError::MissingAuthentication)?;
    Ok(Uuid::parse_str(token)
        .map_err(|_| AuthError::InvalidAuthentication)?
        .as_hyphenated()
        .to_string())
}

fn browser_principal_from_token(token: String) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        id: token.clone(),
        kind: AuthenticatedPrincipalKind::BrowserSession,
        subject: token,
    }
}

pub fn validate_csrf(headers: &HeaderMap) -> Result<(), AuthError> {
    let expected = cookie_value(headers, CSRF_COOKIE_NAME)
        .ok_or(AuthError::MissingCsrfToken)
        .and_then(normalize_uuid_token)?;
    let actual = headers
        .get(CSRF_HEADER_NAME)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(AuthError::MissingCsrfToken)
        .and_then(normalize_uuid_token)?;

    if bool::from(actual.as_bytes().ct_eq(expected.as_bytes())) {
        Ok(())
    } else {
        Err(AuthError::InvalidCsrfToken)
    }
}

fn normalize_uuid_token(value: &str) -> Result<String, AuthError> {
    Uuid::parse_str(value)
        .map(|uuid| uuid.as_hyphenated().to_string())
        .map_err(|_| AuthError::InvalidCsrfToken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header::AUTHORIZATION};

    const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
    const CSRF_TOKEN: &str = "22222222-2222-4222-8222-222222222222";
    const OTHER_CSRF_TOKEN: &str = "33333333-3333-4333-8333-333333333333";

    #[test]
    fn missing_authentication_headers_are_rejected() {
        let error =
            authorize_request(&HeaderMap::new(), false).expect_err("missing auth should fail");

        assert_eq!(error, AuthError::MissingAuthentication);
        assert_eq!(error.message(), "missing authentication");
    }

    #[test]
    fn empty_bearer_tokens_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer   "));

        let error = authorize_request(&headers, false).expect_err("empty bearer token should fail");

        assert_eq!(error, AuthError::InvalidAuthentication);
        assert_eq!(error.message(), "invalid authentication");
    }

    #[test]
    fn valid_bearer_tokens_become_principals() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer developer"));

        let principal = authorize_request(&headers, true).expect("valid bearer token should work");

        assert_eq!(principal.id, "developer");
        assert_eq!(principal.kind, AuthenticatedPrincipalKind::Bearer);
        assert_eq!(principal.subject, "developer");
    }

    #[test]
    fn cookie_authentication_is_supported() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static(
                "theme=dark; acp_session=11111111-1111-4111-8111-111111111111",
            ),
        );

        let principal =
            authorize_request(&headers, false).expect("cookie authentication should work");

        assert_eq!(principal.id, SESSION_ID);
        assert_eq!(principal.kind, AuthenticatedPrincipalKind::BrowserSession);
        assert_eq!(principal.subject, SESSION_ID);
    }

    #[test]
    fn cookie_authentication_rejects_non_uuid_principals() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("acp_session=browser-owner"),
        );

        let error =
            authorize_request(&headers, false).expect_err("invalid cookie auth should fail");

        assert_eq!(error, AuthError::InvalidAuthentication);
    }

    #[test]
    fn cookie_authenticated_posts_require_csrf() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static(
                "acp_session=11111111-1111-4111-8111-111111111111; acp_csrf=22222222-2222-4222-8222-222222222222",
            ),
        );

        let error = authorize_request(&headers, true).expect_err("csrf must be present");

        assert_eq!(error, AuthError::MissingCsrfToken);
    }

    #[test]
    fn cookie_authenticated_posts_reject_mismatched_csrf_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static(
                "acp_session=11111111-1111-4111-8111-111111111111; acp_csrf=22222222-2222-4222-8222-222222222222",
            ),
        );
        headers.insert(CSRF_HEADER_NAME, HeaderValue::from_static(OTHER_CSRF_TOKEN));

        let error = authorize_request(&headers, true).expect_err("csrf mismatches must fail");

        assert_eq!(error, AuthError::InvalidCsrfToken);
    }

    #[test]
    fn cookie_authenticated_posts_accept_matching_csrf_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static(
                "acp_session=11111111-1111-4111-8111-111111111111; acp_csrf=22222222-2222-4222-8222-222222222222",
            ),
        );
        headers.insert(CSRF_HEADER_NAME, HeaderValue::from_static(CSRF_TOKEN));

        let principal =
            authorize_request(&headers, true).expect("matching csrf tokens should succeed");

        assert_eq!(principal.id, SESSION_ID);
        assert_eq!(principal.kind, AuthenticatedPrincipalKind::BrowserSession);
        assert_eq!(principal.subject, SESSION_ID);
    }

    #[test]
    fn cookie_authenticated_posts_reject_non_uuid_csrf_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static(
                "acp_session=11111111-1111-4111-8111-111111111111; acp_csrf=not-a-uuid",
            ),
        );
        headers.insert(CSRF_HEADER_NAME, HeaderValue::from_static("not-a-uuid"));

        let error = authorize_request(&headers, true).expect_err("invalid csrf tokens must fail");

        assert_eq!(error, AuthError::InvalidCsrfToken);
    }

    #[test]
    fn cookie_value_reads_multiple_cookie_headers() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, HeaderValue::from_static("theme=dark"));
        headers.append(
            COOKIE,
            HeaderValue::from_static("acp_session=11111111-1111-4111-8111-111111111111"),
        );

        assert_eq!(
            cookie_value(&headers, SESSION_COOKIE_NAME),
            Some(SESSION_ID)
        );
    }
}
