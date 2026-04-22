use super::{
    InvalidPermissionOptionsSnafu, MockClientError, Result, UnsupportedPermissionOptionsSnafu,
};
use crate::sessions::{PermissionResolutionOutcome, TurnHandle};
use agent_client_protocol::{self as acp, schema};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub(super) struct BackendAcpClient {
    turn: Option<TurnHandle>,
    collected: Arc<Mutex<String>>,
}

impl BackendAcpClient {
    pub(super) fn new(turn: TurnHandle) -> Self {
        Self {
            turn: Some(turn),
            collected: Arc::new(Mutex::new(String::new())),
        }
    }

    pub(super) fn without_turn() -> Self {
        Self {
            turn: None,
            collected: Arc::new(Mutex::new(String::new())),
        }
    }

    pub(super) fn reply_text(&self) -> String {
        self.collected
            .lock()
            .expect("mock reply buffer mutex should not be poisoned")
            .clone()
    }

    pub(super) fn take_reply_text(&self) -> String {
        std::mem::take(
            &mut *self
                .collected
                .lock()
                .expect("mock reply buffer mutex should not be poisoned"),
        )
    }

    pub(super) async fn request_permission(
        &self,
        args: schema::RequestPermissionRequest,
    ) -> acp::Result<schema::RequestPermissionResponse> {
        let turn = self.turn.clone().ok_or_else(acp::Error::internal_error)?;
        let (approve_option_id, deny_option_id) =
            permission_option_ids(&args).map_err(|_| acp::Error::invalid_params())?;
        let summary = args
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| format!("tool {}", args.tool_call.tool_call_id));
        let resolution = turn
            .register_permission_request(summary, approve_option_id, deny_option_id)
            .await
            .map_err(to_acp_error)?;

        match resolution.wait().await {
            PermissionResolutionOutcome::Selected(option_id) => Ok(
                schema::RequestPermissionResponse::new(schema::RequestPermissionOutcome::Selected(
                    schema::SelectedPermissionOutcome::new(option_id),
                )),
            ),
            PermissionResolutionOutcome::Cancelled => Ok(schema::RequestPermissionResponse::new(
                schema::RequestPermissionOutcome::Cancelled,
            )),
        }
    }

    pub(super) async fn session_notification(
        &self,
        args: schema::SessionNotification,
    ) -> acp::Result<()> {
        if let schema::SessionUpdate::AgentMessageChunk(chunk) = args.update {
            self.collected
                .lock()
                .expect("mock reply buffer mutex should not be poisoned")
                .push_str(&content_text(chunk.content));
        }
        Ok(())
    }
}

pub(super) fn content_text(content: schema::ContentBlock) -> String {
    match content {
        schema::ContentBlock::Text(text) => text.text,
        schema::ContentBlock::Image(_) => "<image>".to_string(),
        schema::ContentBlock::Audio(_) => "<audio>".to_string(),
        schema::ContentBlock::ResourceLink(link) => link.uri,
        content => resource_placeholder(matches!(content, schema::ContentBlock::Resource(_))),
    }
}

fn resource_placeholder(is_resource: bool) -> String {
    ["<unsupported>", "<resource>"][usize::from(is_resource)].to_string()
}

pub(super) fn permission_option_ids(
    args: &schema::RequestPermissionRequest,
) -> Result<(String, String), MockClientError> {
    if args.options.iter().any(|option| {
        matches!(
            option.kind,
            schema::PermissionOptionKind::AllowAlways | schema::PermissionOptionKind::RejectAlways
        )
    }) {
        return UnsupportedPermissionOptionsSnafu.fail();
    }

    let approve_option_id = unique_option_id(args, schema::PermissionOptionKind::AllowOnce)?;
    let deny_option_id = unique_option_id(args, schema::PermissionOptionKind::RejectOnce)?;

    match (approve_option_id, deny_option_id) {
        (Some(approve_option_id), Some(deny_option_id)) => Ok((approve_option_id, deny_option_id)),
        _ => InvalidPermissionOptionsSnafu.fail(),
    }
}

fn unique_option_id(
    args: &schema::RequestPermissionRequest,
    kind: schema::PermissionOptionKind,
) -> Result<Option<String>, MockClientError> {
    let mut matches = args
        .options
        .iter()
        .filter(|option| option.kind == kind)
        .map(|option| option.option_id.to_string());
    let first = matches.next();
    if matches.next().is_some() {
        return UnsupportedPermissionOptionsSnafu.fail();
    }
    Ok(first)
}

fn to_acp_error(source: crate::sessions::SessionStoreError) -> acp::Error {
    let _ = source;
    acp::Error::internal_error()
}
