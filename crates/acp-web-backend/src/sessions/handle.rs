use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, broadcast, oneshot, watch};
use uuid::Uuid;

use crate::contract_messages::{ConversationMessage, MessageRole};
use crate::contract_permissions::{
    PermissionDecision, PermissionRequest, ResolvePermissionResponse,
};
use crate::contract_sessions::{SessionListItem, SessionSnapshot, SessionStatus};
use crate::contract_stream::{StreamEvent, StreamEventPayload};

use super::{PendingPermissionResolution, PermissionResolutionOutcome, SessionStoreError};

const SESSION_EVENT_BUFFER_CAPACITY: usize = 1024;

#[derive(Debug)]
pub(super) struct SessionHandle {
    sender: broadcast::Sender<StreamEvent>,
    data: Mutex<SessionData>,
}

#[derive(Debug)]
struct ActiveTurn {
    prompt_order: u64,
    cancel_tx: watch::Sender<bool>,
    cancelled: bool,
    assistant_message_id: Option<String>,
}

#[derive(Debug)]
pub(super) enum PromptCompletion {
    Reply {
        text: String,
        streamed_message_id: Option<String>,
    },
    Status {
        message: String,
        streamed_message_id: Option<String>,
    },
    None,
}

#[derive(Debug)]
struct PendingPermission {
    request_order: u64,
    prompt_order: u64,
    summary: String,
    approve_option_id: String,
    deny_option_id: String,
    outcome_tx: Option<oneshot::Sender<PermissionResolutionOutcome>>,
}

impl PendingPermission {
    fn resolve(mut self, outcome: PermissionResolutionOutcome) {
        if let Some(outcome_tx) = self.outcome_tx.take() {
            let _ = outcome_tx.send(outcome);
        }
    }
}

#[derive(Debug)]
struct SessionData {
    id: String,
    owner: String,
    workspace_id: String,
    title: String,
    title_is_auto: bool,
    status: SessionStatus,
    closed_at: Option<DateTime<Utc>>,
    last_activity_at: DateTime<Utc>,
    recent_order: u64,
    latest_sequence: u64,
    next_prompt_order: u64,
    next_permission_request_id: u64,
    next_completion_order: u64,
    pending_completions: BTreeMap<u64, PromptCompletion>,
    pending_permissions: HashMap<String, PendingPermission>,
    active_turn: Option<ActiveTurn>,
    runtime_unavailable_reason: Option<String>,
    messages: Vec<ConversationMessage>,
}

impl SessionHandle {
    pub(super) fn new(
        id: String,
        owner: String,
        workspace_id: String,
        last_activity_at: DateTime<Utc>,
        recent_order: u64,
    ) -> Self {
        let (sender, _) = broadcast::channel(SESSION_EVENT_BUFFER_CAPACITY);
        Self {
            sender,
            data: Mutex::new(SessionData {
                id,
                owner,
                workspace_id,
                title: "New chat".to_string(),
                title_is_auto: true,
                status: SessionStatus::Active,
                closed_at: None,
                last_activity_at,
                recent_order,
                latest_sequence: 0,
                next_prompt_order: 0,
                next_permission_request_id: 1,
                next_completion_order: 0,
                pending_completions: BTreeMap::new(),
                pending_permissions: HashMap::new(),
                active_turn: None,
                runtime_unavailable_reason: None,
                messages: Vec::new(),
            }),
        }
    }

    pub(super) fn restore(
        owner: String,
        snapshot: SessionSnapshot,
        last_activity_at: DateTime<Utc>,
        recent_order: u64,
    ) -> Self {
        let (sender, _) = broadcast::channel(SESSION_EVENT_BUFFER_CAPACITY);
        let closed_at = if snapshot.status == SessionStatus::Closed {
            Some(last_activity_at)
        } else {
            None
        };
        let title_is_auto = snapshot.title == "New chat"
            && !snapshot.messages.iter().any(Self::user_message_exists);
        let restored_prompt_order = Self::restored_prompt_order(&snapshot.messages);

        Self {
            sender,
            data: Mutex::new(SessionData {
                id: snapshot.id,
                owner,
                workspace_id: snapshot.workspace_id,
                title: snapshot.title,
                title_is_auto,
                status: snapshot.status,
                closed_at,
                last_activity_at,
                recent_order,
                latest_sequence: snapshot.latest_sequence,
                next_prompt_order: restored_prompt_order,
                next_permission_request_id: 1,
                next_completion_order: restored_prompt_order,
                pending_completions: BTreeMap::new(),
                pending_permissions: HashMap::new(),
                active_turn: None,
                runtime_unavailable_reason: None,
                messages: snapshot.messages,
            }),
        }
    }

    pub(super) async fn owner_matches(&self, owner: &str) -> bool {
        self.data.lock().await.owner == owner
    }

    pub(super) async fn owner(&self) -> String {
        self.data.lock().await.owner.clone()
    }

    pub(super) async fn workspace_matches(&self, workspace_id: &str) -> bool {
        self.data.lock().await.workspace_id == workspace_id
    }

    pub(super) async fn is_active(&self) -> bool {
        self.data.lock().await.status == SessionStatus::Active
    }

    #[cfg(test)]
    pub(super) async fn is_turn_active(&self, prompt_order: u64) -> bool {
        self.data
            .lock()
            .await
            .active_turn
            .as_ref()
            .is_some_and(|active_turn| active_turn.prompt_order == prompt_order)
    }

    pub(super) async fn snapshot(&self) -> SessionSnapshot {
        let data = self.data.lock().await;
        SessionSnapshot {
            id: data.id.clone(),
            workspace_id: data.workspace_id.clone(),
            title: data.title.clone(),
            status: data.status.clone(),
            latest_sequence: data.latest_sequence,
            messages: data.messages.clone(),
            pending_permissions: collect_pending_permissions(&data),
            active_turn: data.active_turn.is_some(),
        }
    }

    pub(super) async fn pending_permissions(&self) -> Vec<PermissionRequest> {
        let data = self.data.lock().await;
        collect_pending_permissions(&data)
    }

    pub(super) async fn session_list_item_with_order(&self) -> (SessionListItem, u64) {
        let data = self.data.lock().await;
        (
            SessionListItem {
                id: data.id.clone(),
                workspace_id: data.workspace_id.clone(),
                title: data.title.clone(),
                status: data.status.clone(),
                last_activity_at: data.last_activity_at,
            },
            data.recent_order,
        )
    }

    pub(super) async fn append_message(
        &self,
        role: MessageRole,
        text: String,
    ) -> Result<(), SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }
        let _ = Self::message_event(&mut data, role, text);
        Ok(())
    }

    pub(super) async fn closed_at(&self) -> Option<DateTime<Utc>> {
        self.data.lock().await.closed_at
    }

    pub(super) async fn touch_recent_activity(&self, recent_order: u64) {
        let mut data = self.data.lock().await;
        data.last_activity_at = Utc::now();
        data.recent_order = recent_order;
    }

    pub(super) async fn rename(&self, title: String) {
        let mut data = self.data.lock().await;
        data.title = title;
        data.title_is_auto = false;
    }

    pub(super) async fn submit_user_prompt(
        &self,
        text: String,
    ) -> Result<(StreamEvent, u64), SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }
        if data.runtime_unavailable_reason.is_some() {
            return Err(SessionStoreError::RuntimeUnavailable);
        }

        // Auto-title from the first user prompt when the title has not been manually set.
        if data.title_is_auto
            && !data
                .messages
                .iter()
                .any(|m| matches!(m.role, MessageRole::User))
            && let Some(auto_title) = auto_title_from_prompt(&text)
        {
            data.title = auto_title;
        }

        let prompt_order = data.next_prompt_order;
        data.next_prompt_order += 1;
        let event = Self::message_event(&mut data, MessageRole::User, text);
        Ok((event, prompt_order))
    }

    pub(super) async fn start_turn(
        &self,
        prompt_order: u64,
    ) -> Result<watch::Receiver<bool>, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }
        if data.runtime_unavailable_reason.is_some() {
            return Err(SessionStoreError::RuntimeUnavailable);
        }

        let (cancel_tx, cancel_rx) = watch::channel(false);
        data.active_turn = Some(ActiveTurn {
            prompt_order,
            cancel_tx,
            cancelled: false,
            assistant_message_id: None,
        });
        Ok(cancel_rx)
    }

    pub(super) async fn stream_assistant_chunk(
        &self,
        prompt_order: u64,
        text: String,
    ) -> Result<Option<StreamEvent>, SessionStoreError> {
        if text.is_empty() {
            return Ok(None);
        }

        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }
        Ok(Self::stream_assistant_chunk_locked(
            &mut data,
            prompt_order,
            text,
        ))
    }

    pub(super) async fn register_permission_request(
        &self,
        prompt_order: u64,
        summary: String,
        approve_option_id: String,
        deny_option_id: String,
    ) -> Result<(Option<StreamEvent>, PendingPermissionResolution), SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }

        let Some(active_turn) = data.active_turn.as_ref() else {
            return Ok((None, PendingPermissionResolution::cancelled()));
        };
        if active_turn.prompt_order != prompt_order || active_turn.cancelled {
            return Ok((None, PendingPermissionResolution::cancelled()));
        }

        let request_order = data.next_permission_request_id;
        let request_id = format!("req_{request_order}");
        data.next_permission_request_id += 1;
        let (outcome_tx, outcome_rx) = oneshot::channel();
        data.pending_permissions.insert(
            request_id.clone(),
            PendingPermission {
                request_order,
                prompt_order,
                summary: summary.clone(),
                approve_option_id,
                deny_option_id,
                outcome_tx: Some(outcome_tx),
            },
        );
        data.latest_sequence += 1;

        Ok((
            Some(StreamEvent {
                sequence: data.latest_sequence,
                payload: StreamEventPayload::PermissionRequested {
                    request: PermissionRequest {
                        request_id,
                        summary,
                    },
                },
            }),
            PendingPermissionResolution { outcome_rx },
        ))
    }

    pub(super) async fn resolve_permission(
        &self,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<ResolvePermissionResponse, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }

        let pending = data
            .pending_permissions
            .remove(request_id)
            .ok_or(SessionStoreError::PermissionNotFound)?;
        let option_id = match decision {
            PermissionDecision::Approve => pending.approve_option_id.clone(),
            PermissionDecision::Deny => pending.deny_option_id.clone(),
        };
        pending.resolve(PermissionResolutionOutcome::Selected(option_id));

        Ok(ResolvePermissionResponse {
            request_id: request_id.to_string(),
            decision,
        })
    }

    pub(super) async fn cancel_active_turn(&self) -> Result<bool, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }
        Ok(Self::cancel_active_turn_locked(&mut data))
    }

    pub(super) async fn complete_prompt(
        &self,
        prompt_order: u64,
        completion: PromptCompletion,
    ) -> Result<Vec<StreamEvent>, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }

        let streamed_message_id = Self::streamed_message_id_for_prompt(&data, prompt_order);
        Self::clear_turn_state_locked(&mut data, prompt_order);
        let completion = Self::completion_with_streamed_message(completion, streamed_message_id);
        data.pending_completions.insert(prompt_order, completion);

        Ok(Self::drain_pending_completion_events(&mut data))
    }

    fn completion_with_streamed_message(
        completion: PromptCompletion,
        streamed_message_id: Option<String>,
    ) -> PromptCompletion {
        match completion {
            PromptCompletion::Reply { text, .. } => PromptCompletion::Reply {
                text,
                streamed_message_id,
            },
            PromptCompletion::None if streamed_message_id.is_some() => PromptCompletion::Reply {
                text: String::new(),
                streamed_message_id,
            },
            PromptCompletion::Status { message, .. } => PromptCompletion::Status {
                message,
                streamed_message_id,
            },
            completion => completion,
        }
    }

    fn drain_pending_completion_events(data: &mut SessionData) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        while let Some(completion) = Self::next_pending_completion(data) {
            Self::push_completion_events(data, completion, &mut events);
        }
        events
    }

    fn next_pending_completion(data: &mut SessionData) -> Option<PromptCompletion> {
        let next_completion_order = data.next_completion_order;
        let completion = data.pending_completions.remove(&next_completion_order)?;
        data.next_completion_order += 1;
        Some(completion)
    }

    fn push_completion_events(
        data: &mut SessionData,
        completion: PromptCompletion,
        events: &mut Vec<StreamEvent>,
    ) {
        match completion {
            PromptCompletion::Reply {
                text,
                streamed_message_id,
            } => events.extend(Self::complete_reply_events(data, text, streamed_message_id)),
            PromptCompletion::Status {
                message,
                streamed_message_id,
            } => Self::push_status_completion_events(data, events, message, streamed_message_id),
            PromptCompletion::None => {}
        }
    }

    fn push_status_completion_events(
        data: &mut SessionData,
        events: &mut Vec<StreamEvent>,
        message: String,
        streamed_message_id: Option<String>,
    ) {
        if streamed_message_id.is_some() {
            events.extend(Self::complete_reply_events(
                data,
                String::new(),
                streamed_message_id,
            ));
        }
        events.push(Self::status_event(data, message));
    }

    pub(super) async fn mark_runtime_unavailable(
        &self,
        reason: String,
    ) -> Result<(), SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }
        Self::cancel_all_turns_locked(&mut data);
        data.runtime_unavailable_reason = Some(reason);
        Ok(())
    }

    fn message_event(data: &mut SessionData, role: MessageRole, text: String) -> StreamEvent {
        data.latest_sequence += 1;
        let message = ConversationMessage {
            id: format!("m_{}", Uuid::new_v4().simple()),
            role,
            text,
            created_at: Utc::now(),
        };
        data.messages.push(message.clone());

        StreamEvent {
            sequence: data.latest_sequence,
            payload: StreamEventPayload::ConversationMessage {
                message,
                partial: false,
            },
        }
    }

    fn stream_assistant_chunk_locked(
        data: &mut SessionData,
        prompt_order: u64,
        text: String,
    ) -> Option<StreamEvent> {
        let active_turn = data.active_turn.as_ref()?;
        if active_turn.prompt_order != prompt_order || active_turn.cancelled {
            return None;
        }

        let streamed_message_id = active_turn.assistant_message_id.clone();
        let message = match Self::assistant_message_mut(data, streamed_message_id.as_deref()) {
            Some(message) => {
                message.text.push_str(&text);
                message.clone()
            }
            None => {
                let message = ConversationMessage {
                    id: format!("m_{}", Uuid::new_v4().simple()),
                    role: MessageRole::Assistant,
                    text,
                    created_at: Utc::now(),
                };
                let message_id = message.id.clone();
                data.messages.push(message.clone());
                if let Some(active_turn) = data.active_turn.as_mut() {
                    active_turn.assistant_message_id = Some(message_id);
                }
                message
            }
        };
        Some(Self::conversation_message_event(data, message, true))
    }

    fn complete_reply_events(
        data: &mut SessionData,
        text: String,
        streamed_message_id: Option<String>,
    ) -> Vec<StreamEvent> {
        let Some(message_id) = streamed_message_id else {
            return vec![Self::message_event(data, MessageRole::Assistant, text)];
        };
        let Some(message) = Self::assistant_message_mut(data, Some(&message_id)) else {
            return vec![Self::message_event(data, MessageRole::Assistant, text)];
        };
        if !text.is_empty() && message.text != text {
            message.text = text;
        }
        let message = message.clone();
        vec![Self::conversation_message_event(data, message, false)]
    }

    fn assistant_message_mut<'a>(
        data: &'a mut SessionData,
        message_id: Option<&str>,
    ) -> Option<&'a mut ConversationMessage> {
        let message_id = message_id?;
        data.messages.iter_mut().find(|message| {
            message.id == message_id && matches!(message.role, MessageRole::Assistant)
        })
    }

    fn conversation_message_event(
        data: &mut SessionData,
        message: ConversationMessage,
        partial: bool,
    ) -> StreamEvent {
        data.latest_sequence += 1;
        StreamEvent {
            sequence: data.latest_sequence,
            payload: StreamEventPayload::ConversationMessage { message, partial },
        }
    }

    fn streamed_message_id_for_prompt(data: &SessionData, prompt_order: u64) -> Option<String> {
        data.active_turn
            .as_ref()
            .filter(|active_turn| active_turn.prompt_order == prompt_order)
            .and_then(|active_turn| active_turn.assistant_message_id.clone())
    }

    fn status_event(data: &mut SessionData, message: String) -> StreamEvent {
        data.latest_sequence += 1;
        StreamEvent::status(data.latest_sequence, message)
    }

    pub(super) async fn close(&self, reason: &str) -> Result<StreamEvent, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }

        Self::cancel_all_turns_locked(&mut data);
        data.status = SessionStatus::Closed;
        data.closed_at = Some(Utc::now());
        data.pending_completions.clear();
        data.latest_sequence += 1;

        Ok(StreamEvent {
            sequence: data.latest_sequence,
            payload: StreamEventPayload::SessionClosed {
                session_id: data.id.clone(),
                reason: reason.to_string(),
            },
        })
    }

    pub(super) async fn prepare_delete(&self) {
        let mut data = self.data.lock().await;
        Self::cancel_all_turns_locked(&mut data);
        data.pending_completions.clear();
        if data.status != SessionStatus::Closed {
            data.status = SessionStatus::Closed;
            data.closed_at = Some(Utc::now());
        } else if data.closed_at.is_none() {
            data.closed_at = Some(Utc::now());
        }
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.sender.subscribe()
    }

    pub(super) fn broadcast(&self, event: StreamEvent) {
        let _ = self.sender.send(event);
    }

    fn user_message_exists(message: &ConversationMessage) -> bool {
        matches!(message.role, MessageRole::User)
    }

    fn restored_prompt_order(messages: &[ConversationMessage]) -> u64 {
        messages
            .iter()
            .filter(|message| Self::user_message_exists(message))
            .count() as u64
    }

    fn clear_turn_state_locked(data: &mut SessionData, prompt_order: u64) {
        if data
            .active_turn
            .as_ref()
            .is_some_and(|active_turn| active_turn.prompt_order == prompt_order)
        {
            data.active_turn = None;
        }
        Self::resolve_permissions_for_prompt_locked(
            data,
            prompt_order,
            PermissionResolutionOutcome::Cancelled,
        );
    }

    fn cancel_active_turn_locked(data: &mut SessionData) -> bool {
        let Some(active_turn) = data.active_turn.as_mut() else {
            return false;
        };
        if !active_turn.cancelled {
            active_turn.cancelled = true;
            let _ = active_turn.cancel_tx.send(true);
        }
        let prompt_order = active_turn.prompt_order;
        Self::resolve_permissions_for_prompt_locked(
            data,
            prompt_order,
            PermissionResolutionOutcome::Cancelled,
        );
        true
    }

    fn cancel_all_turns_locked(data: &mut SessionData) {
        if let Some(active_turn) = data.active_turn.as_mut() {
            active_turn.cancelled = true;
            let _ = active_turn.cancel_tx.send(true);
        }
        let prompt_orders = data
            .pending_permissions
            .values()
            .map(|pending| pending.prompt_order)
            .collect::<Vec<_>>();
        for prompt_order in prompt_orders {
            Self::resolve_permissions_for_prompt_locked(
                data,
                prompt_order,
                PermissionResolutionOutcome::Cancelled,
            );
        }
        data.active_turn = None;
    }

    fn resolve_permissions_for_prompt_locked(
        data: &mut SessionData,
        prompt_order: u64,
        outcome: PermissionResolutionOutcome,
    ) {
        let request_ids = data
            .pending_permissions
            .iter()
            .filter(|(_, pending)| pending.prompt_order == prompt_order)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in request_ids {
            if let Some(pending) = data.pending_permissions.remove(&request_id) {
                pending.resolve(outcome.clone());
            }
        }
    }
}

fn auto_title_from_prompt(prompt: &str) -> Option<String> {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = normalized.chars().take(60).collect::<String>();
    (!title.is_empty()).then_some(title)
}

fn collect_pending_permissions(data: &SessionData) -> Vec<PermissionRequest> {
    let mut pending_permissions = data
        .pending_permissions
        .iter()
        .map(|(request_id, pending)| {
            (
                pending.request_order,
                PermissionRequest {
                    request_id: request_id.clone(),
                    summary: pending.summary.clone(),
                },
            )
        })
        .collect::<Vec<_>>();
    pending_permissions.sort_by_key(|(request_order, _)| *request_order);
    pending_permissions
        .into_iter()
        .map(|(_, request)| request)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prepare_delete_sets_closed_at_for_already_closed_sessions() {
        let handle = SessionHandle::new(
            "s_test".to_string(),
            "alice".to_string(),
            "w_test".to_string(),
            Utc::now(),
            1,
        );
        {
            let mut data = handle.data.lock().await;
            data.status = SessionStatus::Closed;
            data.closed_at = None;
        }

        handle.prepare_delete().await;

        let data = handle.data.lock().await;
        assert_eq!(data.status, SessionStatus::Closed);
        assert!(data.closed_at.is_some());
    }
}
