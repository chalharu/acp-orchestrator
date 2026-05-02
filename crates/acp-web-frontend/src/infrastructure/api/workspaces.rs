#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_sessions::{
    AgentProfile, AgentProfileListResponse, AgentProfileMode, AgentProfileResponse,
    CreateSessionRequest, CreateSessionResponse, DeleteAgentProfileResponse, SessionListItem,
    SessionListResponse, UpsertAgentProfileRequest,
};
use acp_contracts_workspaces::{
    CreateWorkspaceRequest, CreateWorkspaceResponse, DeleteWorkspaceResponse,
    UpdateWorkspaceRequest, UpdateWorkspaceResponse, WorkspaceBranch, WorkspaceBranchListResponse,
    WorkspaceDetail, WorkspaceListResponse, WorkspaceSummary,
};
#[cfg(target_family = "wasm")]
use gloo_net::http::Request;

#[cfg(not(target_family = "wasm"))]
use super::encode_component;
#[cfg(target_family = "wasm")]
use super::response_error_message;
#[cfg(target_family = "wasm")]
use super::{csrf_token, encode_component, patch_json_with_csrf, post_json_with_csrf};

const WORKSPACES_URL: &str = "/api/v1/workspaces";
const AGENT_PROFILES_URL: &str = "/api/v1/agent-profiles";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceSessionCreateError {
    NotFound(String),
    Other(String),
}

impl WorkspaceSessionCreateError {
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::NotFound(message) | Self::Other(message) => message,
        }
    }
}

impl std::fmt::Display for WorkspaceSessionCreateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) | Self::Other(message) => formatter.write_str(message),
        }
    }
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_workspaces() -> Result<Vec<WorkspaceSummary>, String> {
    let response = Request::get(WORKSPACES_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List workspaces failed").await);
    }

    let listed: WorkspaceListResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.workspaces)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_workspaces() -> Result<Vec<WorkspaceSummary>, String> {
    Err(non_wasm_api_error("GET", WORKSPACES_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn create_workspace(
    name: &str,
    upstream_url: String,
) -> Result<WorkspaceDetail, String> {
    let body = create_workspace_body(name, upstream_url)?;
    let response = post_json_with_csrf(WORKSPACES_URL, body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Create workspace failed").await);
    }
    let payload: CreateWorkspaceResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.workspace)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_workspace(
    name: &str,
    upstream_url: String,
) -> Result<WorkspaceDetail, String> {
    let _ = create_workspace_body(name, upstream_url)?;
    Err(non_wasm_api_error("POST", WORKSPACES_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn update_workspace(
    workspace_id: &str,
    name: Option<String>,
) -> Result<WorkspaceDetail, String> {
    let body = update_workspace_body(name)?;
    let response = patch_json_with_csrf(&workspace_url(workspace_id), body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Update workspace failed").await);
    }
    let payload: UpdateWorkspaceResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.workspace)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn update_workspace(
    workspace_id: &str,
    name: Option<String>,
) -> Result<WorkspaceDetail, String> {
    let _ = update_workspace_body(name)?;
    Err(non_wasm_api_error("PATCH", &workspace_url(workspace_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn delete_workspace(
    workspace_id: &str,
) -> Result<DeleteWorkspaceResponse, String> {
    let csrf = csrf_token();
    let response = Request::delete(&workspace_url(workspace_id))
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Delete workspace failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn delete_workspace(
    workspace_id: &str,
) -> Result<DeleteWorkspaceResponse, String> {
    Err(non_wasm_api_error("DELETE", &workspace_url(workspace_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn create_workspace_session(
    workspace_id: &str,
    checkout_ref: Option<String>,
    agent_profile_id: Option<String>,
) -> Result<String, WorkspaceSessionCreateError> {
    let response = post_json_with_csrf(
        &workspace_sessions_url(workspace_id),
        create_workspace_session_body(checkout_ref, agent_profile_id)
            .map_err(WorkspaceSessionCreateError::Other)?,
    )
    .await
    .map_err(WorkspaceSessionCreateError::Other)?;
    if response.status() == 404 {
        return Err(WorkspaceSessionCreateError::NotFound(
            response_error_message(response, "Create workspace session failed").await,
        ));
    }
    if !response.ok() {
        return Err(WorkspaceSessionCreateError::Other(
            response_error_message(response, "Create workspace session failed").await,
        ));
    }
    let payload: CreateSessionResponse = response
        .json()
        .await
        .map_err(|error| WorkspaceSessionCreateError::Other(error.to_string()))?;
    Ok(payload.session.id)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_workspace_session(
    workspace_id: &str,
    checkout_ref: Option<String>,
    agent_profile_id: Option<String>,
) -> Result<String, WorkspaceSessionCreateError> {
    let _ = create_workspace_session_body(checkout_ref, agent_profile_id)
        .map_err(WorkspaceSessionCreateError::Other)?;
    Err(WorkspaceSessionCreateError::Other(non_wasm_api_error(
        "POST",
        &workspace_sessions_url(workspace_id),
    )))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_agent_profiles() -> Result<Vec<AgentProfile>, String> {
    let response = Request::get(AGENT_PROFILES_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "List ACP profiles failed").await);
    }
    let listed: AgentProfileListResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.profiles)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_agent_profiles() -> Result<Vec<AgentProfile>, String> {
    Err(non_wasm_api_error("GET", AGENT_PROFILES_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn create_agent_profile(
    name: String,
    command: String,
    mode: AgentProfileMode,
) -> Result<AgentProfile, String> {
    let body = agent_profile_body(name, command, mode)?;
    let csrf = csrf_token();
    let response = Request::post(AGENT_PROFILES_URL)
        .header("x-csrf-token", &csrf)
        .header("content-type", "application/json")
        .body(body)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Save ACP profile failed").await);
    }
    let payload: AgentProfileResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.profile)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn create_agent_profile(
    name: String,
    command: String,
    mode: AgentProfileMode,
) -> Result<AgentProfile, String> {
    let _ = agent_profile_body(name, command, mode)?;
    Err(non_wasm_api_error("POST", AGENT_PROFILES_URL))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn update_agent_profile(
    profile_id: &str,
    name: String,
    command: String,
    mode: AgentProfileMode,
) -> Result<AgentProfile, String> {
    let body = agent_profile_body(name, command, mode)?;
    let response = patch_json_with_csrf(&agent_profile_url(profile_id), body).await?;
    if !response.ok() {
        return Err(response_error_message(response, "Save ACP profile failed").await);
    }
    let payload: AgentProfileResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(payload.profile)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn update_agent_profile(
    profile_id: &str,
    name: String,
    command: String,
    mode: AgentProfileMode,
) -> Result<AgentProfile, String> {
    let _ = agent_profile_body(name, command, mode)?;
    Err(non_wasm_api_error("PATCH", &agent_profile_url(profile_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn delete_agent_profile(
    profile_id: &str,
) -> Result<DeleteAgentProfileResponse, String> {
    let csrf = csrf_token();
    let response = Request::delete(&agent_profile_url(profile_id))
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response_error_message(response, "Delete ACP profile failed").await);
    }
    response.json().await.map_err(|error| error.to_string())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn delete_agent_profile(
    profile_id: &str,
) -> Result<DeleteAgentProfileResponse, String> {
    Err(non_wasm_api_error("DELETE", &agent_profile_url(profile_id)))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_workspace_branches(
    workspace_id: &str,
) -> Result<Vec<WorkspaceBranch>, String> {
    let response = Request::get(&workspace_branches_url(workspace_id))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List workspace branches failed").await);
    }

    let listed: WorkspaceBranchListResponse =
        response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.branches)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_workspace_branches(
    workspace_id: &str,
) -> Result<Vec<WorkspaceBranch>, String> {
    Err(non_wasm_api_error(
        "GET",
        &workspace_branches_url(workspace_id),
    ))
}

#[cfg(target_family = "wasm")]
pub(crate) async fn list_workspace_sessions(
    workspace_id: &str,
) -> Result<Vec<SessionListItem>, String> {
    let response = Request::get(&workspace_sessions_url(workspace_id))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.ok() {
        return Err(response_error_message(response, "List workspace sessions failed").await);
    }

    let listed: SessionListResponse = response.json().await.map_err(|error| error.to_string())?;
    Ok(listed.sessions)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn list_workspace_sessions(
    workspace_id: &str,
) -> Result<Vec<SessionListItem>, String> {
    Err(non_wasm_api_error(
        "GET",
        &workspace_sessions_url(workspace_id),
    ))
}

fn create_workspace_body(name: &str, upstream_url: String) -> Result<String, String> {
    serde_json::to_string(&CreateWorkspaceRequest {
        name: name.to_string(),
        upstream_url: normalize_required_text(upstream_url)?,
        credential_reference_id: None,
    })
    .map_err(|error| error.to_string())
}

fn create_workspace_session_body(
    checkout_ref: Option<String>,
    agent_profile_id: Option<String>,
) -> Result<String, String> {
    serde_json::to_string(&CreateSessionRequest {
        checkout_ref: normalize_optional_text(checkout_ref),
        agent_profile_id: normalize_optional_text(agent_profile_id),
    })
    .map_err(|error| error.to_string())
}

fn agent_profile_body(
    name: String,
    command: String,
    mode: AgentProfileMode,
) -> Result<String, String> {
    let name = normalize_profile_name(name)?;
    let command_argv = command_line_to_argv(&command)?;
    let env_allowlist = agent_profile_env_allowlist(&mode);
    serde_json::to_string(&UpsertAgentProfileRequest {
        name,
        mode,
        command_argv,
        env_allowlist,
        timeout_seconds: 30,
        run_uid: 65_534,
        run_gid: 65_534,
    })
    .map_err(|error| error.to_string())
}

fn agent_profile_env_allowlist(mode: &AgentProfileMode) -> Vec<String> {
    match mode {
        AgentProfileMode::Host => [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "SHELL",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "TMPDIR",
            "TMP",
            "TEMP",
        ]
        .into_iter()
        .map(|name| name.to_string())
        .collect(),
        AgentProfileMode::Chroot => Vec::new(),
    }
}

fn normalize_profile_name(name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Profile name is required".to_string());
    }
    Ok(name)
}

pub(crate) fn command_line_to_argv(command: &str) -> Result<Vec<String>, String> {
    let mut parser = CommandLineParser::default();
    for ch in command.trim().chars() {
        parser.push_char(ch);
    }
    parser.finish()
}

#[derive(Default)]
struct CommandLineParser {
    argv: Vec<String>,
    current: String,
    quote: Option<QuoteMode>,
    escaped: bool,
    started_arg: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QuoteMode {
    Single,
    Double,
}

impl QuoteMode {
    const fn as_char(self) -> char {
        match self {
            Self::Single => '\'',
            Self::Double => '"',
        }
    }
}

impl CommandLineParser {
    fn push_char(&mut self, ch: char) {
        if self.escaped {
            self.push_escaped(ch);
            return;
        }

        match self.quote {
            Some(QuoteMode::Single) => self.push_single_quoted(ch),
            Some(QuoteMode::Double) => self.push_double_quoted(ch),
            None => self.push_unquoted(ch),
        }
    }

    fn push_escaped(&mut self, ch: char) {
        if self.quote == Some(QuoteMode::Double) && ch != '"' && ch != '\\' {
            self.current.push('\\');
        }
        self.push_literal(ch);
        self.escaped = false;
    }

    fn push_single_quoted(&mut self, ch: char) {
        if ch == '\'' {
            self.end_quote();
        } else {
            self.push_literal(ch);
        }
    }

    fn push_double_quoted(&mut self, ch: char) {
        match ch {
            '\\' => self.start_escape(),
            '"' => self.end_quote(),
            _ => self.push_literal(ch),
        }
    }

    fn push_unquoted(&mut self, ch: char) {
        match ch {
            '\\' => self.start_escape(),
            '\'' => self.start_quote(QuoteMode::Single),
            '"' => self.start_quote(QuoteMode::Double),
            _ if ch.is_whitespace() => self.finish_arg(),
            _ => self.push_literal(ch),
        }
    }

    fn start_escape(&mut self) {
        self.escaped = true;
        self.started_arg = true;
    }

    fn start_quote(&mut self, quote: QuoteMode) {
        self.quote = Some(quote);
        self.started_arg = true;
    }

    fn end_quote(&mut self) {
        self.quote = None;
        self.started_arg = true;
    }

    fn push_literal(&mut self, ch: char) {
        self.current.push(ch);
        self.started_arg = true;
    }

    fn finish_arg(&mut self) {
        if self.started_arg {
            self.argv.push(std::mem::take(&mut self.current));
            self.started_arg = false;
        }
    }

    fn finish(mut self) -> Result<Vec<String>, String> {
        if self.escaped {
            return Err("ACP launch command has a trailing backslash escape".to_string());
        }
        if let Some(quote) = self.quote {
            let quote_char = quote.as_char();
            return Err(format!(
                "ACP launch command has an unterminated {quote_char} quote"
            ));
        }
        self.finish_arg();
        if self.argv.is_empty() {
            Err("ACP launch command is required".to_string())
        } else {
            Ok(self.argv)
        }
    }
}

fn update_workspace_body(name: Option<String>) -> Result<String, String> {
    serde_json::to_string(&UpdateWorkspaceRequest { name }).map_err(|error| error.to_string())
}

fn workspace_url(workspace_id: &str) -> String {
    format!("{WORKSPACES_URL}/{}", encode_component(workspace_id))
}

fn agent_profile_url(profile_id: &str) -> String {
    format!("{AGENT_PROFILES_URL}/{}", encode_component(profile_id))
}

fn workspace_sessions_url(workspace_id: &str) -> String {
    format!("{}/sessions", workspace_url(workspace_id))
}

fn workspace_branches_url(workspace_id: &str) -> String {
    format!("{}/branches", workspace_url(workspace_id))
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

fn normalize_required_text(value: String) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err("repository_url is required".to_string());
    }
    Ok(value)
}

#[cfg(not(target_family = "wasm"))]
fn non_wasm_api_error(method: &str, url: &str) -> String {
    format!("Browser {method} workspaces API is unavailable on non-wasm targets: {url}")
}

#[cfg(test)]
mod tests {
    use crate::infrastructure::api::poll_ready;

    use super::*;

    #[test]
    fn workspace_request_bodies_serialize_expected_payloads() {
        let body =
            create_workspace_body("My Workspace", " https://example.com/repo.git ".to_string())
                .expect("create body");
        assert!(body.contains("My Workspace"));
        assert!(body.contains("https://example.com/repo.git"));

        let body = update_workspace_body(Some("Renamed".to_string())).expect("update body");
        assert!(body.contains("Renamed"));

        let body = create_workspace_session_body(
            Some(" refs/heads/release ".to_string()),
            Some(" opencode ".to_string()),
        )
        .expect("body");
        assert!(body.contains("refs/heads/release"));
        assert!(body.contains("opencode"));

        let body = agent_profile_body(
            " Claude ACP ".to_string(),
            r#"claude acp --config "~/Library/Application Support/Claude/config.json" --port ${ACP_PORT}"#
                .to_string(),
            AgentProfileMode::Host,
        )
        .expect("profile body");
        let request: UpsertAgentProfileRequest =
            serde_json::from_str(&body).expect("profile body should decode");
        assert_eq!(request.name, "Claude ACP");
        assert_eq!(request.mode, AgentProfileMode::Host);
        assert!(request.env_allowlist.contains(&"PATH".to_string()));
        assert!(request.env_allowlist.contains(&"HOME".to_string()));
        assert_eq!(
            request.command_argv,
            vec![
                "claude",
                "acp",
                "--config",
                "~/Library/Application Support/Claude/config.json",
                "--port",
                "${ACP_PORT}"
            ]
        );
    }

    #[test]
    fn agent_profile_body_keeps_chroot_profiles_env_explicit() {
        let body = agent_profile_body(
            "Isolated ACP".to_string(),
            "agent acp --port ${ACP_PORT}".to_string(),
            AgentProfileMode::Chroot,
        )
        .expect("profile body");
        let request: UpsertAgentProfileRequest =
            serde_json::from_str(&body).expect("profile body should decode");

        assert_eq!(request.mode, AgentProfileMode::Chroot);
        assert!(request.env_allowlist.is_empty());
    }

    #[test]
    fn workspace_url_appends_workspace_id() {
        assert_eq!(workspace_url("w_123"), "/api/v1/workspaces/w_123");
        assert_eq!(workspace_url("w/1"), "/api/v1/workspaces/w%2F1");
        assert_eq!(
            workspace_sessions_url("w/1"),
            "/api/v1/workspaces/w%2F1/sessions"
        );
        assert_eq!(
            workspace_branches_url("w/1"),
            "/api/v1/workspaces/w%2F1/branches"
        );
        assert_eq!(WORKSPACES_URL, "/api/v1/workspaces");
        assert_eq!(
            agent_profile_url("profile/1"),
            "/api/v1/agent-profiles/profile%2F1"
        );
    }

    #[test]
    fn host_workspace_api_functions_fail_with_descriptive_errors() {
        let list_error = poll_ready(list_workspaces()).expect_err("host list should fail");
        assert!(list_error.contains(WORKSPACES_URL));

        let create_error = poll_ready(create_workspace(
            "test",
            "https://example.com/repo.git".to_string(),
        ))
        .expect_err("host create should fail");
        assert!(create_error.contains(WORKSPACES_URL));

        let update_error = poll_ready(update_workspace("w_1", Some("new".to_string())))
            .expect_err("host update should fail");
        assert!(update_error.contains("/api/v1/workspaces/w_1"));

        let delete_error =
            poll_ready(delete_workspace("w_1")).expect_err("host delete should fail");
        assert!(delete_error.contains("/api/v1/workspaces/w_1"));
    }

    #[test]
    fn host_agent_profile_api_functions_fail_with_descriptive_errors() {
        let profiles_error = poll_ready(list_agent_profiles()).expect_err("host profiles fail");
        assert!(profiles_error.contains(AGENT_PROFILES_URL));

        let save_profile_error = poll_ready(create_agent_profile(
            "OpenCode ACP".to_string(),
            "opencode acp".to_string(),
            AgentProfileMode::Host,
        ))
        .expect_err("host profile save should fail");
        assert!(save_profile_error.contains(AGENT_PROFILES_URL));
        assert!(save_profile_error.contains("POST"));

        let update_profile_error = poll_ready(update_agent_profile(
            "profile_1",
            "OpenCode ACP".to_string(),
            "opencode acp".to_string(),
            AgentProfileMode::Chroot,
        ))
        .expect_err("host profile update should fail");
        assert!(update_profile_error.contains("/api/v1/agent-profiles/profile_1"));
        assert!(update_profile_error.contains("PATCH"));

        let delete_profile_error =
            poll_ready(delete_agent_profile("profile_1")).expect_err("host profile delete");
        assert!(delete_profile_error.contains("/api/v1/agent-profiles/profile_1"));
        assert!(delete_profile_error.contains("DELETE"));
    }

    #[test]
    fn host_workspace_session_api_functions_fail_with_descriptive_errors() {
        let create_session_error = poll_ready(create_workspace_session("w_1", None, None))
            .expect_err("host workspace session create should fail");
        assert!(
            create_session_error
                .to_string()
                .contains("/api/v1/workspaces/w_1/sessions")
        );

        let list_branches_error = poll_ready(list_workspace_branches("w_1"))
            .expect_err("host workspace branches should fail");
        assert!(list_branches_error.contains("/api/v1/workspaces/w_1/branches"));

        let list_sessions_error = poll_ready(list_workspace_sessions("w_1"))
            .expect_err("host workspace session list should fail");
        assert!(list_sessions_error.contains("/api/v1/workspaces/w_1/sessions"));
    }

    #[test]
    fn workspace_session_create_errors_preserve_messages() {
        assert_eq!(
            WorkspaceSessionCreateError::NotFound("missing".to_string()).into_message(),
            "missing"
        );
        assert_eq!(
            WorkspaceSessionCreateError::Other("boom".to_string()).into_message(),
            "boom"
        );
    }

    #[test]
    fn normalize_optional_text_trims_and_drops_blank_values() {
        assert_eq!(
            normalize_optional_text(Some(" value ".to_string())),
            Some("value".to_string())
        );
        assert_eq!(normalize_optional_text(Some("   ".to_string())), None);
        assert_eq!(normalize_optional_text(None), None);
    }

    #[test]
    fn normalize_required_text_trims_and_rejects_blank_values() {
        assert_eq!(
            normalize_required_text(" value ".to_string()).expect("value should trim"),
            "value"
        );
        assert_eq!(
            normalize_required_text("   ".to_string()).expect_err("blank values should fail"),
            "repository_url is required"
        );
    }

    #[test]
    fn command_line_to_argv_parses_plain_and_quoted_commands() {
        assert_eq!(
            command_line_to_argv(" opencode acp --port ${ACP_PORT} ").expect("valid argv"),
            vec!["opencode", "acp", "--port", "${ACP_PORT}"]
        );
        assert_eq!(
            command_line_to_argv(
                r#"claude acp --config "~/Library/Application Support/Claude/config.json" --port ${ACP_PORT}"#
            )
            .expect("quoted argv"),
            vec![
                "claude",
                "acp",
                "--config",
                "~/Library/Application Support/Claude/config.json",
                "--port",
                "${ACP_PORT}"
            ]
        );
    }

    #[test]
    fn command_line_to_argv_parses_escapes() {
        assert_eq!(
            command_line_to_argv(r#"copilot acp --name 'Copilot CLI' --flag escaped\ value"#)
                .expect("escaped argv"),
            vec![
                "copilot",
                "acp",
                "--name",
                "Copilot CLI",
                "--flag",
                "escaped value"
            ]
        );
        assert_eq!(
            command_line_to_argv(r#"agent --path 'dir\name' --quoted "escaped\"quote""#)
                .expect("quoted backslash argv"),
            vec![
                "agent",
                "--path",
                r#"dir\name"#,
                "--quoted",
                "escaped\"quote"
            ]
        );
        assert_eq!(
            command_line_to_argv(r#"agent --regex "\d+\.json" --slash "dir\\name""#)
                .expect("double-quoted backslash argv"),
            vec!["agent", "--regex", r#"\d+\.json"#, "--slash", r#"dir\name"#]
        );
    }

    #[test]
    fn command_line_to_argv_rejects_incomplete_commands() {
        assert!(command_line_to_argv(" \n ").is_err());
        assert!(command_line_to_argv("claude acp \"unterminated").is_err());
        assert!(command_line_to_argv("claude acp 'unterminated").is_err());
        assert!(command_line_to_argv(r"agent trailing\").is_err());
    }

    #[test]
    fn agent_profile_body_requires_a_profile_name() {
        assert_eq!(
            agent_profile_body(
                "   ".to_string(),
                "claude acp".to_string(),
                AgentProfileMode::Host
            )
            .expect_err("blank profile names should fail"),
            "Profile name is required"
        );
    }
}
