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
        .map_err(|_| AuthError::InvalidAuthorization)?
        .trim();
    let token = raw
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
