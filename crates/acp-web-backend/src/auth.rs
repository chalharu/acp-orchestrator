use axum::http::{HeaderMap, header::AUTHORIZATION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    MissingAuthorization,
    InvalidAuthorization,
}

impl AuthError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::MissingAuthorization => "missing bearer token",
            Self::InvalidAuthorization => "invalid bearer token",
        }
    }
}

pub fn extract_principal(headers: &HeaderMap) -> Result<AuthenticatedPrincipal, AuthError> {
    let value = headers
        .get(AUTHORIZATION)
        .ok_or(AuthError::MissingAuthorization)?;
    let raw = value
        .to_str()
        .map_err(|_| AuthError::InvalidAuthorization)?;
    let token = raw
        .trim_start()
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidAuthorization)?
        .trim();

    if token.is_empty() {
        return Err(AuthError::InvalidAuthorization);
    }

    Ok(AuthenticatedPrincipal {
        id: token.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header::AUTHORIZATION};

    #[test]
    fn missing_authorization_headers_are_rejected() {
        let error = extract_principal(&HeaderMap::new()).expect_err("missing auth should fail");

        assert_eq!(error, AuthError::MissingAuthorization);
        assert_eq!(error.message(), "missing bearer token");
    }

    #[test]
    fn empty_bearer_tokens_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer   "));

        let error = extract_principal(&headers).expect_err("empty bearer token should fail");

        assert_eq!(error, AuthError::InvalidAuthorization);
        assert_eq!(error.message(), "invalid bearer token");
    }

    #[test]
    fn valid_bearer_tokens_become_principals() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer developer"));

        let principal = extract_principal(&headers).expect("valid bearer token should succeed");

        assert_eq!(principal.id, "developer");
    }
}
