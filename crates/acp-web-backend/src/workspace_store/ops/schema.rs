use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{auth::AuthenticatedPrincipalKind, workspace_records::WorkspaceStoreError};

use super::{
    LEGACY_BROWSER_SESSIONS_TABLE, LOCAL_ACCOUNT_PRINCIPAL_KIND,
    accounts::durable_local_account_subject,
    queries::{load_active_local_account_by_username, load_user_by_principal},
    shared::database_error,
};

const WORKSPACE_STORE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    user_id TEXT PRIMARY KEY,
    principal_kind TEXT NOT NULL,
    principal_subject TEXT NOT NULL,
    username TEXT,
    password_hash TEXT,
    is_admin INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    deleted_at TEXT,
    UNIQUE(principal_kind, principal_subject)
);

CREATE TABLE IF NOT EXISTS browser_sessions (
    browser_session_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    deleted_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(user_id)
);

CREATE INDEX IF NOT EXISTS browser_sessions_user_id_idx
    ON browser_sessions(user_id);

CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    upstream_url TEXT,
    default_ref TEXT,
    credential_reference_id TEXT,
    status TEXT NOT NULL,
    bootstrap_kind TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    FOREIGN KEY (owner_user_id) REFERENCES users(user_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS workspaces_owner_bootstrap_kind_idx
    ON workspaces(owner_user_id, bootstrap_kind)
    WHERE bootstrap_kind IS NOT NULL;

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    checkout_relpath TEXT,
    checkout_ref TEXT,
    checkout_commit_sha TEXT,
    failure_reason TEXT,
    detach_deadline_at TEXT,
    restartable_deadline_at TEXT,
    created_at TEXT NOT NULL,
    last_activity_at TEXT NOT NULL,
    latest_sequence INTEGER NOT NULL DEFAULT 0,
    messages_json TEXT NOT NULL DEFAULT '[]',
    closed_at TEXT,
    deleted_at TEXT,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(workspace_id),
    FOREIGN KEY (owner_user_id) REFERENCES users(user_id)
);

CREATE INDEX IF NOT EXISTS sessions_owner_user_id_idx
    ON sessions(owner_user_id);

CREATE INDEX IF NOT EXISTS sessions_workspace_id_idx
    ON sessions(workspace_id);
"#;

const BROWSER_SESSIONS_REBUILD_TABLE_SQL: &str = r#"
CREATE TABLE {table_name} (
    browser_session_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    deleted_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(user_id)
)
"#;

const WORKSPACES_REBUILD_TABLE_SQL: &str = r#"
CREATE TABLE {table_name} (
    workspace_id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    upstream_url TEXT,
    default_ref TEXT,
    credential_reference_id TEXT,
    status TEXT NOT NULL,
    bootstrap_kind TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    FOREIGN KEY (owner_user_id) REFERENCES users(user_id)
)
"#;

const SESSIONS_REBUILD_TABLE_SQL: &str = r#"
CREATE TABLE {table_name} (
    session_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    checkout_relpath TEXT,
    checkout_ref TEXT,
    checkout_commit_sha TEXT,
    failure_reason TEXT,
    detach_deadline_at TEXT,
    restartable_deadline_at TEXT,
    created_at TEXT NOT NULL,
    last_activity_at TEXT NOT NULL,
    latest_sequence INTEGER NOT NULL DEFAULT 0,
    messages_json TEXT NOT NULL DEFAULT '[]',
    closed_at TEXT,
    deleted_at TEXT,
    FOREIGN KEY (workspace_id) REFERENCES workspaces(workspace_id),
    FOREIGN KEY (owner_user_id) REFERENCES users(user_id)
)
"#;

#[derive(Clone, Copy)]
struct ExpectedForeignKey {
    from_column: &'static str,
    parent_table: &'static str,
    parent_column: &'static str,
}

#[derive(Clone, Copy)]
struct TableRebuildColumn {
    name: &'static str,
    fallback_sql: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForeignKeyReference {
    from_column: String,
    parent_table: String,
    parent_column: String,
}

const BROWSER_SESSIONS_FOREIGN_KEYS: &[ExpectedForeignKey] = &[ExpectedForeignKey {
    from_column: "user_id",
    parent_table: "users",
    parent_column: "user_id",
}];

const WORKSPACES_FOREIGN_KEYS: &[ExpectedForeignKey] = &[ExpectedForeignKey {
    from_column: "owner_user_id",
    parent_table: "users",
    parent_column: "user_id",
}];

const SESSIONS_FOREIGN_KEYS: &[ExpectedForeignKey] = &[
    ExpectedForeignKey {
        from_column: "workspace_id",
        parent_table: "workspaces",
        parent_column: "workspace_id",
    },
    ExpectedForeignKey {
        from_column: "owner_user_id",
        parent_table: "users",
        parent_column: "user_id",
    },
];

const BROWSER_SESSIONS_REBUILD_COLUMNS_SPEC: &str = "\
browser_session_id
user_id
created_at
last_seen_at
deleted_at=NULL
";

const WORKSPACES_REBUILD_COLUMNS_SPEC: &str = "\
workspace_id
owner_user_id
name
upstream_url=NULL
default_ref=NULL
credential_reference_id=NULL
status
bootstrap_kind=NULL
created_at
updated_at
deleted_at=NULL
";

const SESSIONS_REBUILD_COLUMNS_SPEC: &str = "\
session_id
workspace_id
owner_user_id
title
status
checkout_relpath=NULL
checkout_ref=NULL
checkout_commit_sha=NULL
failure_reason=NULL
detach_deadline_at=NULL
restartable_deadline_at=NULL
created_at
last_activity_at
latest_sequence=0
messages_json='[]'
closed_at=NULL
deleted_at=NULL
";

fn parse_rebuild_columns(spec: &'static str) -> Vec<TableRebuildColumn> {
    spec.lines()
        .filter(|line| !line.is_empty())
        .map(|line| match line.split_once('=') {
            Some((name, fallback_sql)) => TableRebuildColumn {
                name,
                fallback_sql: Some(fallback_sql),
            },
            None => TableRebuildColumn {
                name: line,
                fallback_sql: None,
            },
        })
        .collect()
}

fn ensure_table_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
    column_definition: &str,
) -> Result<(), WorkspaceStoreError> {
    let columns = table_columns(connection, table_name)?;
    if columns.iter().any(|column| column == column_name) {
        return Ok(());
    }

    connection
        .execute(
            &format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_definition}"),
            [],
        )
        .map_err(database_error)?;
    Ok(())
}

fn ensure_users_column(
    connection: &Connection,
    column_name: &str,
    column_definition: &str,
) -> Result<(), WorkspaceStoreError> {
    ensure_table_column(connection, "users", column_name, column_definition)
}

fn ensure_sessions_column(
    connection: &Connection,
    column_name: &str,
    column_definition: &str,
) -> Result<(), WorkspaceStoreError> {
    ensure_table_column(connection, "sessions", column_name, column_definition)
}

pub(in crate::workspace_store) fn initialize_schema(
    connection: &Connection,
) -> Result<(), WorkspaceStoreError> {
    stage_legacy_browser_sessions_table(connection)?;
    connection
        .execute_batch(WORKSPACE_STORE_SCHEMA_SQL)
        .map_err(database_error)?;
    ensure_user_auth_columns(connection)?;
    ensure_session_snapshot_columns(connection)?;
    migrate_legacy_auth_schema(connection)?;
    prune_orphaned_foreign_key_rows(connection)?;
    ensure_foreign_key_tables(connection)?;
    recreate_users_username_index(connection)?;
    ensure_foreign_key_integrity(connection)?;
    Ok(())
}

fn prune_orphaned_foreign_key_rows(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    connection
        .execute_batch(
            "DELETE FROM workspaces
             WHERE NOT EXISTS (
                 SELECT 1
                 FROM users
                 WHERE users.user_id = workspaces.owner_user_id
             );

             DELETE FROM sessions
             WHERE NOT EXISTS (
                     SELECT 1
                     FROM workspaces
                     WHERE workspaces.workspace_id = sessions.workspace_id
                 )
                OR NOT EXISTS (
                     SELECT 1
                     FROM users
                     WHERE users.user_id = sessions.owner_user_id
                 );

             DELETE FROM browser_sessions
             WHERE NOT EXISTS (
                 SELECT 1
                 FROM users
                 WHERE users.user_id = browser_sessions.user_id
             );",
        )
        .map_err(database_error)?;
    Ok(())
}

#[rustfmt::skip]
fn ensure_foreign_key_tables(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    let browser_session_columns = parse_rebuild_columns(BROWSER_SESSIONS_REBUILD_COLUMNS_SPEC);
    let workspace_columns = parse_rebuild_columns(WORKSPACES_REBUILD_COLUMNS_SPEC);
    let session_columns = parse_rebuild_columns(SESSIONS_REBUILD_COLUMNS_SPEC);
    rebuild_table_with_foreign_keys_if_needed(connection, "browser_sessions", BROWSER_SESSIONS_REBUILD_TABLE_SQL, &browser_session_columns, BROWSER_SESSIONS_FOREIGN_KEYS)?;
    rebuild_table_with_foreign_keys_if_needed(connection, "workspaces", WORKSPACES_REBUILD_TABLE_SQL, &workspace_columns, WORKSPACES_FOREIGN_KEYS)?;
    rebuild_table_with_foreign_keys_if_needed(connection, "sessions", SESSIONS_REBUILD_TABLE_SQL, &session_columns, SESSIONS_FOREIGN_KEYS)?;
    connection
        .execute_batch(WORKSPACE_STORE_SCHEMA_SQL)
        .map_err(database_error)?;
    Ok(())
}

fn rebuild_table_with_foreign_keys_if_needed(
    connection: &Connection,
    table_name: &str,
    create_table_sql_template: &str,
    columns: &[TableRebuildColumn],
    expected_foreign_keys: &[ExpectedForeignKey],
) -> Result<(), WorkspaceStoreError> {
    if !table_exists(connection, table_name)?
        || table_has_expected_foreign_keys(connection, table_name, expected_foreign_keys)?
    {
        return Ok(());
    }

    let existing_columns = table_columns(connection, table_name)?;
    let temp_table_name = format!("__acp_rebuild_{table_name}");
    let insert_columns = columns
        .iter()
        .map(|column| column.name)
        .collect::<Vec<_>>()
        .join(", ");
    let select_columns = columns
        .iter()
        .map(|column| rebuild_select_expression(table_name, &existing_columns, *column))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let create_table_sql = create_table_sql_template.replace("{table_name}", &temp_table_name);

    connection
        .execute_batch(&format!(
            "DROP TABLE IF EXISTS {temp_table_name};
             {create_table_sql};
             INSERT INTO {temp_table_name} ({insert_columns})
             SELECT {select_columns}
             FROM {table_name};
             DROP TABLE {table_name};
             ALTER TABLE {temp_table_name} RENAME TO {table_name};"
        ))
        .map_err(database_error)?;
    Ok(())
}

fn rebuild_select_expression(
    table_name: &str,
    existing_columns: &[String],
    column: TableRebuildColumn,
) -> Result<String, WorkspaceStoreError> {
    if existing_columns
        .iter()
        .any(|existing| existing == column.name)
    {
        return Ok(column.name.to_string());
    }

    column.fallback_sql.map(str::to_string).ok_or_else(|| {
        WorkspaceStoreError::Database(format!(
            "table '{table_name}' is missing required column '{}'",
            column.name
        ))
    })
}

fn table_has_expected_foreign_keys(
    connection: &Connection,
    table_name: &str,
    expected_foreign_keys: &[ExpectedForeignKey],
) -> Result<bool, WorkspaceStoreError> {
    let actual_foreign_keys = table_foreign_keys(connection, table_name)?;
    Ok(actual_foreign_keys.len() == expected_foreign_keys.len()
        && expected_foreign_keys.iter().all(|expected| {
            actual_foreign_keys.iter().any(|actual| {
                actual.from_column == expected.from_column
                    && actual.parent_table == expected.parent_table
                    && actual.parent_column == expected.parent_column
            })
        }))
}

fn table_foreign_keys(
    connection: &Connection,
    table_name: &str,
) -> Result<Vec<ForeignKeyReference>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA foreign_key_list({table_name})"))
        .map_err(database_error)?;
    statement
        .query_map([], |row| {
            Ok(ForeignKeyReference {
                parent_table: row.get(2)?,
                from_column: row.get(3)?,
                parent_column: row.get(4)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn ensure_foreign_key_integrity(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(database_error)?;
    let violations = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    if violations.is_empty() {
        return Ok(());
    }

    let details = violations
        .into_iter()
        .map(|(table, row_id, parent, fk_index)| {
            format!(
                "{table}(rowid={}) -> {parent} [fk#{fk_index}]",
                row_id
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "NULL".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(WorkspaceStoreError::Database(format!(
        "foreign key check failed: {details}"
    )))
}

fn ensure_user_auth_columns(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    ensure_users_column(connection, "username", "TEXT")?;
    ensure_users_column(connection, "password_hash", "TEXT")?;
    ensure_users_column(connection, "is_admin", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_users_column(connection, "deleted_at", "TEXT")?;
    Ok(())
}

fn ensure_session_snapshot_columns(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    ensure_sessions_column(connection, "latest_sequence", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_sessions_column(connection, "messages_json", "TEXT NOT NULL DEFAULT '[]'")?;
    Ok(())
}

fn migrate_legacy_auth_schema(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    promote_legacy_bearer_admins(connection)?;
    migrate_legacy_local_accounts(connection)?;
    migrate_legacy_browser_sessions(connection)?;
    drop_legacy_auth_tables(connection)?;
    Ok(())
}

fn recreate_users_username_index(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS users_username_idx;
                 CREATE UNIQUE INDEX IF NOT EXISTS users_username_idx
                    ON users(username)
                    WHERE username IS NOT NULL AND deleted_at IS NULL;",
        )
        .map_err(database_error)?;
    Ok(())
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool, WorkspaceStoreError> {
    connection
        .query_row(
            "SELECT 1
             FROM sqlite_master
             WHERE type = 'table' AND name = ?1",
            params![table_name],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(database_error)
}

fn table_columns(
    connection: &Connection,
    table_name: &str,
) -> Result<Vec<String>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .map_err(database_error)?;
    statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn table_has_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, WorkspaceStoreError> {
    Ok(table_columns(connection, table_name)?
        .iter()
        .any(|column| column == column_name))
}

fn stage_legacy_browser_sessions_table(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    if !table_exists(connection, "browser_sessions")?
        || table_has_column(connection, "browser_sessions", "user_id")?
    {
        return Ok(());
    }

    if table_exists(connection, LEGACY_BROWSER_SESSIONS_TABLE)? {
        return Err(WorkspaceStoreError::Database(format!(
            "legacy browser sessions table '{LEGACY_BROWSER_SESSIONS_TABLE}' already exists"
        )));
    }

    connection
        .execute(
            "ALTER TABLE browser_sessions RENAME TO legacy_browser_sessions",
            [],
        )
        .map_err(database_error)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct LegacyLocalAccountRecord {
    username: String,
    password_hash: String,
    is_admin: bool,
    created_at: String,
    updated_at: String,
}

fn load_legacy_local_accounts(
    connection: &Connection,
) -> Result<Option<(Vec<LegacyLocalAccountRecord>, bool)>, WorkspaceStoreError> {
    if !table_exists(connection, "local_accounts")? {
        return Ok(None);
    }

    let has_is_admin = table_has_column(connection, "local_accounts", "is_admin")?;
    let accounts = query_legacy_local_accounts(connection, has_is_admin)?;
    Ok(Some((accounts, has_is_admin)))
}

fn query_legacy_local_accounts(
    connection: &Connection,
    has_is_admin: bool,
) -> Result<Vec<LegacyLocalAccountRecord>, WorkspaceStoreError> {
    let mut statement = connection
        .prepare(legacy_local_accounts_select_sql(has_is_admin))
        .map_err(database_error)?;
    statement
        .query_map([], legacy_local_account_from_row)
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn legacy_local_accounts_select_sql(has_is_admin: bool) -> &'static str {
    if has_is_admin {
        "SELECT user_name, password_hash, is_admin, created_at, updated_at
         FROM local_accounts
         ORDER BY created_at ASC, user_name ASC"
    } else {
        "SELECT user_name, password_hash, 0, created_at, updated_at
         FROM local_accounts
         ORDER BY created_at ASC, user_name ASC"
    }
}

fn legacy_local_account_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<LegacyLocalAccountRecord> {
    Ok(LegacyLocalAccountRecord {
        username: row.get(0)?,
        password_hash: row.get(1)?,
        is_admin: row.get::<_, i64>(2)? != 0,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn migrate_legacy_local_accounts(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    let Some((accounts, has_is_admin)) = load_legacy_local_accounts(connection)? else {
        return Ok(());
    };
    let promote_oldest = !has_is_admin || !accounts.iter().any(|account| account.is_admin);

    for (index, account) in accounts.iter().enumerate() {
        let is_admin = account.is_admin || (promote_oldest && index == 0);
        let principal_subject = durable_local_account_subject(&account.username);
        if load_user_by_principal(connection, LOCAL_ACCOUNT_PRINCIPAL_KIND, &principal_subject)?
            .is_some()
        {
            continue;
        }

        connection
            .execute(
                "INSERT INTO users (
                    user_id,
                    principal_kind,
                    principal_subject,
                    username,
                    password_hash,
                    is_admin,
                    created_at,
                    last_seen_at,
                    deleted_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                params![
                    format!("u_{}", Uuid::new_v4().simple()),
                    LOCAL_ACCOUNT_PRINCIPAL_KIND,
                    principal_subject,
                    &account.username,
                    &account.password_hash,
                    if is_admin { 1 } else { 0 },
                    &account.created_at,
                    &account.updated_at,
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct LegacyBrowserSessionRecord {
    browser_session_id: String,
    principal_subject: String,
    created_at: String,
    last_seen_at: String,
}

fn load_legacy_browser_sessions(
    connection: &Connection,
) -> Result<Vec<LegacyBrowserSessionRecord>, WorkspaceStoreError> {
    if !table_exists(connection, LEGACY_BROWSER_SESSIONS_TABLE)? {
        return Ok(Vec::new());
    }

    let mut statement = connection
        .prepare(
            "SELECT session_token, principal_subject, created_at, last_seen_at
             FROM legacy_browser_sessions",
        )
        .map_err(database_error)?;
    statement
        .query_map([], |row| {
            Ok(LegacyBrowserSessionRecord {
                browser_session_id: row.get(0)?,
                principal_subject: row.get(1)?,
                created_at: row.get(2)?,
                last_seen_at: row.get(3)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

fn migrate_legacy_browser_sessions(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    for session in load_legacy_browser_sessions(connection)? {
        let Some(user) =
            load_active_local_account_by_username(connection, &session.principal_subject)?
        else {
            continue;
        };

        connection
            .execute(
                "INSERT INTO browser_sessions (
                    browser_session_id,
                    user_id,
                    created_at,
                    last_seen_at,
                    deleted_at
                 ) VALUES (?1, ?2, ?3, ?4, NULL)
                 ON CONFLICT(browser_session_id) DO UPDATE SET
                    user_id = excluded.user_id,
                    created_at = excluded.created_at,
                    last_seen_at = excluded.last_seen_at,
                    deleted_at = NULL",
                params![
                    session.browser_session_id,
                    user.user_id,
                    session.created_at,
                    session.last_seen_at,
                ],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

fn drop_legacy_auth_tables(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    connection
        .execute("DROP TABLE IF EXISTS local_accounts", [])
        .map_err(database_error)?;
    connection
        .execute("DROP TABLE IF EXISTS legacy_browser_sessions", [])
        .map_err(database_error)?;
    Ok(())
}

fn promote_legacy_bearer_admins(connection: &Connection) -> Result<(), WorkspaceStoreError> {
    connection
        .execute(
            "UPDATE users
             SET is_admin = 1
             WHERE principal_kind = ?1
               AND deleted_at IS NULL",
            params![AuthenticatedPrincipalKind::Bearer.as_str()],
        )
        .map_err(database_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TIMESTAMP: &str = "2026-04-27T00:00:00Z";

    fn test_connection() -> Connection {
        Connection::open_in_memory().expect("in-memory sqlite should open")
    }

    fn prepare_users_only_schema(connection: &Connection) {
        connection
            .execute_batch(WORKSPACE_STORE_SCHEMA_SQL)
            .expect("workspace schema should initialize");
        connection
            .execute_batch(
                "DROP TABLE sessions;
                 DROP TABLE browser_sessions;
                 DROP TABLE workspaces;",
            )
            .expect("workspace tables should drop");
    }

    fn create_legacy_workspace_tables(connection: &Connection) {
        for sql in [
            "CREATE TABLE browser_sessions (browser_session_id TEXT PRIMARY KEY, user_id TEXT NOT NULL, created_at TEXT NOT NULL, last_seen_at TEXT NOT NULL);",
            "CREATE TABLE workspaces (workspace_id TEXT PRIMARY KEY, owner_user_id TEXT NOT NULL, name TEXT NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);",
            "CREATE TABLE sessions (session_id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, owner_user_id TEXT NOT NULL, title TEXT NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL, last_activity_at TEXT NOT NULL);",
        ] {
            connection
                .execute_batch(sql)
                .expect("legacy table should initialize");
        }
    }

    fn insert_user(connection: &Connection, user_id: &str) {
        connection
            .execute(
                "INSERT INTO users VALUES (?1, 'bearer', 'developer', NULL, NULL, 1, ?2, ?2, NULL)",
                params![user_id, TEST_TIMESTAMP],
            )
            .expect("user should insert");
    }

    fn insert_legacy_workspace_rows(connection: &Connection, owner_user_id: &str) {
        connection
            .execute(
                "INSERT INTO browser_sessions VALUES (?1, ?2, ?3, ?3)",
                params!["bs_1", owner_user_id, TEST_TIMESTAMP],
            )
            .expect("browser session should insert");
        connection
            .execute(
                "INSERT INTO workspaces VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params!["w_1", owner_user_id, "Workspace", "ready", TEST_TIMESTAMP],
            )
            .expect("workspace should insert");
        connection
            .execute(
                "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    "s_1",
                    "w_1",
                    owner_user_id,
                    "Session",
                    "active",
                    TEST_TIMESTAMP
                ],
            )
            .expect("session should insert");
    }

    fn assert_foreign_keys(
        connection: &Connection,
        table_name: &str,
        expected_foreign_keys: &[ExpectedForeignKey],
    ) {
        assert!(
            table_has_expected_foreign_keys(connection, table_name, expected_foreign_keys)
                .expect("foreign keys should load"),
            "{table_name} should include the expected foreign keys"
        );
    }

    fn create_parent_child_tables(connection: &Connection) {
        connection
            .execute_batch("CREATE TABLE parent (id TEXT PRIMARY KEY);")
            .expect("parent table should initialize");
        connection
            .execute_batch(
                "CREATE TABLE child (id TEXT PRIMARY KEY, parent_id TEXT NOT NULL, FOREIGN KEY (parent_id) REFERENCES parent(id));",
            )
            .expect("child table should initialize");
    }

    fn insert_parent(connection: &Connection, parent_id: &str) {
        connection
            .execute("INSERT INTO parent VALUES (?1)", params![parent_id])
            .expect("parent row should insert");
    }

    fn insert_child(connection: &Connection, child_id: &str, parent_id: &str) {
        connection
            .execute(
                "INSERT INTO child VALUES (?1, ?2)",
                params![child_id, parent_id],
            )
            .expect("child row should insert");
    }

    #[test]
    fn ensure_foreign_key_tables_rebuilds_all_workspace_tables() {
        let connection = test_connection();
        prepare_users_only_schema(&connection);
        create_legacy_workspace_tables(&connection);
        insert_user(&connection, "u_owner");
        insert_legacy_workspace_rows(&connection, "u_owner");

        ensure_foreign_key_tables(&connection).expect("tables should rebuild with foreign keys");

        assert_foreign_keys(
            &connection,
            "browser_sessions",
            BROWSER_SESSIONS_FOREIGN_KEYS,
        );
        assert_foreign_keys(&connection, "workspaces", WORKSPACES_FOREIGN_KEYS);
        assert_foreign_keys(&connection, "sessions", SESSIONS_FOREIGN_KEYS);
        let snapshot_defaults: (i64, String) = connection
            .query_row(
                "SELECT latest_sequence, messages_json FROM sessions WHERE session_id = 's_1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("session defaults should survive rebuild");
        assert_eq!(snapshot_defaults, (0, "[]".to_string()));
    }

    #[test]
    fn rebuild_select_expression_rejects_missing_required_columns() {
        let error = rebuild_select_expression(
            "sessions",
            &[],
            TableRebuildColumn {
                name: "session_id",
                fallback_sql: None,
            },
        )
        .expect_err("missing required columns should fail");

        assert_eq!(
            error,
            WorkspaceStoreError::Database(
                "table 'sessions' is missing required column 'session_id'".to_string()
            )
        );
    }

    #[test]
    fn ensure_foreign_key_integrity_accepts_clean_tables_and_reports_violations() {
        let connection = test_connection();
        create_parent_child_tables(&connection);
        insert_parent(&connection, "p_1");
        insert_child(&connection, "c_valid", "p_1");

        ensure_foreign_key_integrity(&connection).expect("clean schema should pass integrity");

        connection
            .pragma_update(None, "foreign_keys", false)
            .expect("foreign key pragma should disable");
        insert_child(&connection, "c_orphan", "p_missing");

        let error =
            ensure_foreign_key_integrity(&connection).expect_err("orphan rows should be reported");
        assert_eq!(
            error,
            WorkspaceStoreError::Database(
                "foreign key check failed: child(rowid=2) -> parent [fk#0]".to_string()
            )
        );
    }
}
