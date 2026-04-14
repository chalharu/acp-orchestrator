use super::{
    InvalidPermissionOptionsSnafu, MockClientError, Result, UnsupportedPermissionOptionsSnafu,
};
use crate::sessions::{PermissionResolutionOutcome, TurnHandle};
use agent_client_protocol as acp;
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone)]
pub(super) struct BackendAcpClient {
    turn: TurnHandle,
    collected: Rc<RefCell<String>>,
}

impl BackendAcpClient {
    pub(super) fn new(turn: TurnHandle) -> Self {
        Self {
            turn,
            collected: Rc::new(RefCell::new(String::new())),
        }
    }

    pub(super) fn reply_text(&self) -> String {
        self.collected.borrow().clone()
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for BackendAcpClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        let (approve_option_id, deny_option_id) =
            permission_option_ids(&args).map_err(|_| acp::Error::invalid_params())?;
        let summary = args
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| format!("tool {}", args.tool_call.tool_call_id));
        let resolution = self
            .turn
            .register_permission_request(summary, approve_option_id, deny_option_id)
            .await
            .map_err(to_acp_error)?;

        match resolution.wait().await {
            PermissionResolutionOutcome::Selected(option_id) => Ok(
                acp::RequestPermissionResponse::new(acp::RequestPermissionOutcome::Selected(
                    acp::SelectedPermissionOutcome::new(option_id),
                )),
            ),
            PermissionResolutionOutcome::Cancelled => Ok(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            )),
        }
    }

    async fn session_notification(
        &self,
        args: acp::SessionNotification,
    ) -> acp::Result<(), acp::Error> {
        if let acp::SessionUpdate::AgentMessageChunk(chunk) = args.update {
            self.collected
                .borrow_mut()
                .push_str(&content_text(chunk.content));
        }
        Ok(())
    }
}

pub(super) fn content_text(content: acp::ContentBlock) -> String {
    match content {
        acp::ContentBlock::Text(text) => text.text,
        acp::ContentBlock::Image(_) => "<image>".to_string(),
        acp::ContentBlock::Audio(_) => "<audio>".to_string(),
        acp::ContentBlock::ResourceLink(link) => link.uri,
        content => resource_placeholder(matches!(content, acp::ContentBlock::Resource(_))),
    }
}

fn resource_placeholder(is_resource: bool) -> String {
    ["<unsupported>", "<resource>"][usize::from(is_resource)].to_string()
}

pub(super) fn permission_option_ids(
    args: &acp::RequestPermissionRequest,
) -> Result<(String, String), MockClientError> {
    if args.options.iter().any(|option| {
        matches!(
            option.kind,
            acp::PermissionOptionKind::AllowAlways | acp::PermissionOptionKind::RejectAlways
        )
    }) {
        return UnsupportedPermissionOptionsSnafu.fail();
    }

    let approve_option_id = unique_option_id(args, acp::PermissionOptionKind::AllowOnce)?;
    let deny_option_id = unique_option_id(args, acp::PermissionOptionKind::RejectOnce)?;

    match (approve_option_id, deny_option_id) {
        (Some(approve_option_id), Some(deny_option_id)) => Ok((approve_option_id, deny_option_id)),
        _ => InvalidPermissionOptionsSnafu.fail(),
    }
}

fn unique_option_id(
    args: &acp::RequestPermissionRequest,
    kind: acp::PermissionOptionKind,
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
