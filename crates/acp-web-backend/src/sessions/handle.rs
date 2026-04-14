use std::collections::{BTreeMap, HashMap};

use acp_contracts::{
    ConversationMessage, MessageRole, PermissionDecision, PermissionRequest,
    ResolvePermissionResponse, SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, broadcast, oneshot, watch};
use uuid::Uuid;

use super::{PendingPermissionResolution, PermissionResolutionOutcome, SessionStoreError};

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
}

#[derive(Debug)]
pub(super) enum PromptCompletion {
    Reply(String),
    Status(String),
    None,
}

#[derive(Debug)]
struct PendingPermission {
    prompt_order: u64,
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
    status: SessionStatus,
    closed_at: Option<DateTime<Utc>>,
    latest_sequence: u64,
    next_prompt_order: u64,
    next_permission_request_id: u64,
    next_completion_order: u64,
    pending_completions: BTreeMap<u64, PromptCompletion>,
    pending_permissions: HashMap<String, PendingPermission>,
    active_turn: Option<ActiveTurn>,
    messages: Vec<ConversationMessage>,
}

impl SessionHandle {
    pub(super) fn new(id: String, owner: String) -> Self {
        let (sender, _) = broadcast::channel(64);
        Self {
            sender,
            data: Mutex::new(SessionData {
                id,
                owner,
                status: SessionStatus::Active,
                closed_at: None,
                latest_sequence: 0,
                next_prompt_order: 0,
                next_permission_request_id: 1,
                next_completion_order: 0,
                pending_completions: BTreeMap::new(),
                pending_permissions: HashMap::new(),
                active_turn: None,
                messages: Vec::new(),
            }),
        }
    }

    pub(super) async fn owner_matches(&self, owner: &str) -> bool {
        self.data.lock().await.owner == owner
    }

    pub(super) async fn is_active(&self) -> bool {
        self.data.lock().await.status == SessionStatus::Active
    }

    pub(super) async fn snapshot(&self) -> SessionSnapshot {
        let data = self.data.lock().await;
        SessionSnapshot {
            id: data.id.clone(),
            status: data.status.clone(),
            latest_sequence: data.latest_sequence,
            messages: data.messages.clone(),
        }
    }

    pub(super) async fn closed_at(&self) -> Option<DateTime<Utc>> {
        self.data.lock().await.closed_at
    }

    pub(super) async fn submit_user_prompt(
        &self,
        text: String,
    ) -> Result<(StreamEvent, u64), SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
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

        let (cancel_tx, cancel_rx) = watch::channel(false);
        data.active_turn = Some(ActiveTurn {
            prompt_order,
            cancel_tx,
            cancelled: false,
        });
        Ok(cancel_rx)
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

        let request_id = format!("req_{}", data.next_permission_request_id);
        data.next_permission_request_id += 1;
        let (outcome_tx, outcome_rx) = oneshot::channel();
        data.pending_permissions.insert(
            request_id.clone(),
            PendingPermission {
                prompt_order,
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

        Self::clear_turn_state_locked(&mut data, prompt_order);
        data.pending_completions.insert(prompt_order, completion);
        let mut events = Vec::new();
        loop {
            let next_completion_order = data.next_completion_order;
            let Some(completion) = data.pending_completions.remove(&next_completion_order) else {
                break;
            };
            data.next_completion_order += 1;
            match completion {
                PromptCompletion::Reply(text) => {
                    events.push(Self::message_event(&mut data, MessageRole::Assistant, text));
                }
                PromptCompletion::Status(message) => {
                    events.push(Self::status_event(&mut data, message));
                }
                PromptCompletion::None => {}
            }
        }

        Ok(events)
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
            payload: StreamEventPayload::ConversationMessage { message },
        }
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

    pub(super) fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.sender.subscribe()
    }

    pub(super) fn broadcast(&self, event: StreamEvent) {
        let _ = self.sender.send(event);
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
