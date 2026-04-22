use std::{fmt, fmt::Write as _, path::Path};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::{
    auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind},
    workspace_records::WorkspaceStoreError,
};

pub(super) fn ensure_parent_dir(path: &Path) -> Result<(), WorkspaceStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| WorkspaceStoreError::Io(format!("create state directory: {error}")))?;
    }
    Ok(())
}

pub(super) fn open_immediate_transaction(
    connection: &mut Connection,
) -> Result<rusqlite::Transaction<'_>, WorkspaceStoreError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)
}

pub(super) fn timestamp(value: &DateTime<Utc>) -> String {
    value.to_rfc3339()
}

pub(super) fn parse_timestamp(value: String) -> Result<DateTime<Utc>, WorkspaceStoreError> {
    DateTime::parse_from_rfc3339(&value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            WorkspaceStoreError::Database(format!("invalid timestamp '{value}': {error}"))
        })
}

fn parse_optional_timestamp(
    value: Option<String>,
) -> Result<Option<DateTime<Utc>>, WorkspaceStoreError> {
    value.map(parse_timestamp).transpose()
}

pub(super) fn parse_timestamp_for_row(
    value: String,
    index: usize,
) -> rusqlite::Result<DateTime<Utc>> {
    parse_timestamp(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

pub(super) fn parse_optional_timestamp_for_row(
    value: Option<String>,
    index: usize,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    parse_optional_timestamp(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

pub(super) fn database_error(error: impl fmt::Display) -> WorkspaceStoreError {
    WorkspaceStoreError::Database(error.to_string())
}

pub(super) fn join_error(error: tokio::task::JoinError) -> WorkspaceStoreError {
    WorkspaceStoreError::Database(format!("blocking workspace task failed: {error}"))
}

pub(super) fn durable_principal_subject(principal: &AuthenticatedPrincipal) -> String {
    hash_subject(principal.kind.as_str(), &principal.subject)
}

pub(super) fn hash_subject(namespace: &str, subject: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.as_bytes());
    digest.update([0]);
    digest.update(subject.as_bytes());
    let digest = digest.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

impl AuthenticatedPrincipalKind {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::BrowserSession => "browser_session",
        }
    }
}
