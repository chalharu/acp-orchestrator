use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use acp_contracts::{
    ConversationMessage, MessageRole, PermissionDecision, PermissionRequest,
    ResolvePermissionResponse, SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot, watch};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions: Arc<RwLock<HashMap<String, Arc<SessionHandle>>>>,
    create_session_lock: Arc<Mutex<()>>,
    closed_session_limit: usize,
    session_cap: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStoreError {
    NotFound,
    Forbidden,
    Closed,
    EmptyPrompt,
    PermissionNotFound,
    SessionCapReached,
}

impl SessionStoreError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotFound => "session not found",
            Self::Forbidden => "session owner mismatch",
            Self::Closed => "session already closed",
            Self::EmptyPrompt => "prompt must not be empty",
            Self::PermissionNotFound => "permission request not found",
            Self::SessionCapReached => "session cap reached for principal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PermissionResolutionOutcome {
    Selected(String),
    Cancelled,
}

#[derive(Debug)]
pub(crate) struct PendingPermissionResolution {
    outcome_rx: oneshot::Receiver<PermissionResolutionOutcome>,
}

impl PendingPermissionResolution {
    fn cancelled() -> Self {
        let (outcome_tx, outcome_rx) = oneshot::channel();
        let _ = outcome_tx.send(PermissionResolutionOutcome::Cancelled);
        Self { outcome_rx }
    }

    pub async fn wait(self) -> PermissionResolutionOutcome {
        self.outcome_rx
            .await
            .unwrap_or(PermissionResolutionOutcome::Cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct PendingPrompt {
    handle: Arc<SessionHandle>,
    session_id: String,
    prompt_text: String,
    prompt_order: u64,
}

impl PendingPrompt {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn prompt_text(&self) -> &str {
        &self.prompt_text
    }

    pub(crate) async fn start_turn(&self) -> Result<watch::Receiver<bool>, SessionStoreError> {
        self.handle.start_turn(self.prompt_order).await
    }

    pub(crate) async fn register_permission_request(
        &self,
        summary: String,
        approve_option_id: String,
        deny_option_id: String,
    ) -> Result<PendingPermissionResolution, SessionStoreError> {
        let (event, resolution) = self
            .handle
            .register_permission_request(
                self.prompt_order,
                summary,
                approve_option_id,
                deny_option_id,
            )
            .await?;
        if let Some(event) = event {
            self.handle.broadcast(event);
        }
        Ok(resolution)
    }

    pub async fn complete_with_reply(self, text: String) {
        if let Ok(events) = self
            .handle
            .complete_prompt(self.prompt_order, PromptCompletion::Reply(text))
            .await
        {
            for event in events {
                self.handle.broadcast(event);
            }
        }
    }

    pub async fn complete_with_status(self, message: impl Into<String>) {
        if let Ok(events) = self
            .handle
            .complete_prompt(self.prompt_order, PromptCompletion::Status(message.into()))
            .await
        {
            for event in events {
                self.handle.broadcast(event);
            }
        }
    }

    pub async fn complete_without_output(self) {
        if let Ok(events) = self
            .handle
            .complete_prompt(self.prompt_order, PromptCompletion::None)
            .await
        {
            for event in events {
                self.handle.broadcast(event);
            }
        }
    }
}

impl SessionStore {
    pub fn new(session_cap: usize) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            create_session_lock: Arc::new(Mutex::new(())),
            closed_session_limit: 32,
            session_cap,
        }
    }

    pub async fn create_session(&self, owner: &str) -> Result<SessionSnapshot, SessionStoreError> {
        let _guard = self.create_session_lock.lock().await;
        let handles = {
            let sessions = self.sessions.read().await;
            sessions.values().cloned().collect::<Vec<_>>()
        };

        let mut active_sessions = 0usize;
        for handle in handles {
            if handle.owner_matches(owner).await && handle.is_active().await {
                active_sessions += 1;
            }
        }

        if active_sessions >= self.session_cap {
            return Err(SessionStoreError::SessionCapReached);
        }

        let session_id = format!("s_{}", Uuid::new_v4().simple());
        let handle = Arc::new(SessionHandle::new(session_id.clone(), owner.to_string()));
        let snapshot = handle.snapshot().await;

        self.sessions.write().await.insert(session_id, handle);

        Ok(snapshot)
    }

    pub async fn session_snapshot(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<SessionSnapshot, SessionStoreError> {
        let handle = self.authorized_handle(owner, session_id).await?;
        Ok(handle.snapshot().await)
    }

    pub async fn session_history(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<Vec<ConversationMessage>, SessionStoreError> {
        let handle = self.authorized_handle(owner, session_id).await?;
        Ok(handle.snapshot().await.messages)
    }

    pub async fn session_events(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<(SessionSnapshot, broadcast::Receiver<StreamEvent>), SessionStoreError> {
        let handle = self.authorized_handle(owner, session_id).await?;
        let snapshot = handle.snapshot().await;
        let receiver = handle.subscribe();
        Ok((snapshot, receiver))
    }

    pub async fn submit_prompt(
        &self,
        owner: &str,
        session_id: &str,
        text: String,
    ) -> Result<PendingPrompt, SessionStoreError> {
        if text.trim().is_empty() {
            return Err(SessionStoreError::EmptyPrompt);
        }

        let handle = self.authorized_handle(owner, session_id).await?;
        let (user_event, prompt_order) = handle.submit_user_prompt(text.clone()).await?;
        handle.broadcast(user_event);

        Ok(PendingPrompt {
            handle,
            session_id: session_id.to_string(),
            prompt_text: text,
            prompt_order,
        })
    }

    pub async fn resolve_permission(
        &self,
        owner: &str,
        session_id: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<ResolvePermissionResponse, SessionStoreError> {
        let handle = self.authorized_handle(owner, session_id).await?;
        handle.resolve_permission(request_id, decision).await
    }

    pub async fn cancel_active_turn(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<bool, SessionStoreError> {
        let handle = self.authorized_handle(owner, session_id).await?;
        handle.cancel_active_turn().await
    }

    pub async fn close_session(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<SessionSnapshot, SessionStoreError> {
        let handle = self.authorized_handle(owner, session_id).await?;
        let close_event = handle.close("closed by user").await?;
        handle.broadcast(close_event);
        let snapshot = handle.snapshot().await;
        self.prune_closed_sessions().await;
        Ok(snapshot)
    }

    async fn authorized_handle(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<Arc<SessionHandle>, SessionStoreError> {
        let handle = {
            let sessions = self.sessions.read().await;
            sessions
                .get(session_id)
                .cloned()
                .ok_or(SessionStoreError::NotFound)?
        };

        if !handle.owner_matches(owner).await {
            return Err(SessionStoreError::Forbidden);
        }

        Ok(handle)
    }

    async fn prune_closed_sessions(&self) {
        let handles = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .map(|(session_id, handle)| (session_id.clone(), handle.clone()))
                .collect::<Vec<_>>()
        };

        let mut closed_sessions = Vec::new();
        for (session_id, handle) in handles {
            if let Some(closed_at) = handle.closed_at().await {
                closed_sessions.push((session_id, closed_at));
            }
        }

        if closed_sessions.len() <= self.closed_session_limit {
            return;
        }

        closed_sessions.sort_by(|left, right| right.1.cmp(&left.1));
        let stale_session_ids = closed_sessions
            .into_iter()
            .skip(self.closed_session_limit)
            .map(|(session_id, _)| session_id)
            .collect::<Vec<_>>();

        let mut sessions = self.sessions.write().await;
        for session_id in stale_session_ids {
            sessions.remove(&session_id);
        }
    }
}

#[derive(Debug)]
struct SessionHandle {
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
enum PromptCompletion {
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
    fn new(id: String, owner: String) -> Self {
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

    async fn owner_matches(&self, owner: &str) -> bool {
        self.data.lock().await.owner == owner
    }

    async fn is_active(&self) -> bool {
        self.data.lock().await.status == SessionStatus::Active
    }

    async fn snapshot(&self) -> SessionSnapshot {
        let data = self.data.lock().await;
        SessionSnapshot {
            id: data.id.clone(),
            status: data.status.clone(),
            latest_sequence: data.latest_sequence,
            messages: data.messages.clone(),
        }
    }

    async fn closed_at(&self) -> Option<DateTime<Utc>> {
        self.data.lock().await.closed_at
    }

    async fn submit_user_prompt(
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

    async fn start_turn(
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

    async fn register_permission_request(
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

    async fn resolve_permission(
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

    async fn cancel_active_turn(&self) -> Result<bool, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }
        Ok(Self::cancel_active_turn_locked(&mut data))
    }

    async fn complete_prompt(
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

    async fn close(&self, reason: &str) -> Result<StreamEvent, SessionStoreError> {
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

    fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.sender.subscribe()
    }

    fn broadcast(&self, event: StreamEvent) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_history_includes_completed_replies() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "hello".to_string())
            .await
            .expect("prompt submission should succeed");

        pending.complete_with_reply("hello back".to_string()).await;

        let history = store
            .session_history("alice", &session.id)
            .await
            .expect("session history should load");

        assert_eq!(history.len(), 2);
        assert!(matches!(history[0].role, MessageRole::User));
        assert_eq!(history[0].text, "hello");
        assert!(matches!(history[1].role, MessageRole::Assistant));
        assert_eq!(history[1].text, "hello back");
    }

    #[tokio::test]
    async fn assistant_replies_follow_prompt_submission_order() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let first = store
            .submit_prompt("alice", &session.id, "first".to_string())
            .await
            .expect("first prompt submission should succeed");
        let second = store
            .submit_prompt("alice", &session.id, "second".to_string())
            .await
            .expect("second prompt submission should succeed");

        second
            .complete_with_reply("reply for second".to_string())
            .await;

        let history = store
            .session_history("alice", &session.id)
            .await
            .expect("session history should load");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].text, "first");
        assert_eq!(history[1].text, "second");

        first
            .complete_with_reply("reply for first".to_string())
            .await;

        let history = store
            .session_history("alice", &session.id)
            .await
            .expect("session history should load");
        assert_eq!(history.len(), 4);
        assert!(matches!(history[2].role, MessageRole::Assistant));
        assert_eq!(history[2].text, "reply for first");
        assert!(matches!(history[3].role, MessageRole::Assistant));
        assert_eq!(history[3].text, "reply for second");
    }

    #[tokio::test]
    async fn pending_permission_resolutions_can_default_to_cancelled() {
        assert_eq!(
            PendingPermissionResolution::cancelled().wait().await,
            PermissionResolutionOutcome::Cancelled
        );
    }

    #[tokio::test]
    async fn complete_without_output_releases_queued_follow_up_events() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let (_snapshot, mut receiver) = store
            .session_events("alice", &session.id)
            .await
            .expect("subscribing should succeed");
        let first = store
            .submit_prompt("alice", &session.id, "first".to_string())
            .await
            .expect("first prompt submission should succeed");
        let second = store
            .submit_prompt("alice", &session.id, "second".to_string())
            .await
            .expect("second prompt submission should succeed");

        let _ = receiver
            .recv()
            .await
            .expect("first user event should arrive");
        let _ = receiver
            .recv()
            .await
            .expect("second user event should arrive");

        second
            .complete_with_reply("reply for second".to_string())
            .await;
        let no_assistant =
            tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await;
        assert!(
            no_assistant.is_err(),
            "later replies should stay queued until earlier prompts complete"
        );

        first.complete_without_output().await;

        let assistant_event = receiver
            .recv()
            .await
            .expect("queued assistant reply should be broadcast");
        assert!(matches!(
            assistant_event.payload,
            StreamEventPayload::ConversationMessage { message }
                if matches!(message.role, MessageRole::Assistant)
                    && message.text == "reply for second"
        ));
    }

    #[tokio::test]
    async fn complete_without_output_is_ignored_after_session_close() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let (_snapshot, mut receiver) = store
            .session_events("alice", &session.id)
            .await
            .expect("subscribing should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "hello".to_string())
            .await
            .expect("prompt submission should succeed");

        let _ = receiver.recv().await.expect("user event should arrive");
        store
            .close_session("alice", &session.id)
            .await
            .expect("closing the session should succeed");
        let _ = receiver.recv().await.expect("close event should arrive");

        pending.complete_without_output().await;

        let no_follow_up =
            tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await;
        assert!(
            no_follow_up.is_err(),
            "closed sessions should ignore pending silent completions"
        );
    }

    #[tokio::test]
    async fn pending_replies_are_ignored_after_session_close() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "hello".to_string())
            .await
            .expect("prompt submission should succeed");

        store
            .close_session("alice", &session.id)
            .await
            .expect("closing the session should succeed");
        pending.complete_with_reply("late reply".to_string()).await;

        let history = store
            .session_history("alice", &session.id)
            .await
            .expect("session history should load");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].text, "hello");
    }

    #[tokio::test]
    async fn empty_prompts_are_rejected() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");

        let error = store
            .submit_prompt("alice", &session.id, "   ".to_string())
            .await
            .expect_err("empty prompt should fail");

        assert_eq!(error, SessionStoreError::EmptyPrompt);
        assert_eq!(error.message(), "prompt must not be empty");
    }

    #[tokio::test]
    async fn pending_prompts_can_broadcast_status_updates() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let (_snapshot, mut receiver) = store
            .session_events("alice", &session.id)
            .await
            .expect("subscribing should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "hello".to_string())
            .await
            .expect("prompt submission should succeed");

        let user_event = receiver.recv().await.expect("user event should arrive");
        assert!(matches!(
            user_event.payload,
            StreamEventPayload::ConversationMessage { message }
                if matches!(message.role, MessageRole::User)
        ));

        pending.complete_with_status("mock request failed").await;

        let status_event = receiver.recv().await.expect("status event should arrive");
        assert!(matches!(
            status_event.payload,
            StreamEventPayload::Status { message } if message == "mock request failed"
        ));
    }

    #[tokio::test]
    async fn pending_status_updates_are_ignored_after_session_close() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let (_snapshot, mut receiver) = store
            .session_events("alice", &session.id)
            .await
            .expect("subscribing should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "hello".to_string())
            .await
            .expect("prompt submission should succeed");

        let _ = receiver.recv().await.expect("user event should arrive");
        let _ = store
            .close_session("alice", &session.id)
            .await
            .expect("closing should succeed");
        let closed_event = receiver.recv().await.expect("close event should arrive");
        assert!(matches!(
            closed_event.payload,
            StreamEventPayload::SessionClosed { .. }
        ));

        pending.complete_with_status("should be ignored").await;

        let no_follow_up =
            tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await;
        assert!(
            no_follow_up.is_err(),
            "no extra status event should be broadcast"
        );
    }

    #[tokio::test]
    async fn permission_requests_can_be_resolved_for_the_active_turn() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let (_snapshot, mut receiver) = store
            .session_events("alice", &session.id)
            .await
            .expect("subscribing should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "permission please".to_string())
            .await
            .expect("prompt submission should succeed");

        let _ = receiver.recv().await.expect("user event should arrive");
        let _cancel_rx = pending
            .start_turn()
            .await
            .expect("starting the turn should succeed");
        let resolution = pending
            .register_permission_request(
                "read_text_file README.md".to_string(),
                "allow_once".to_string(),
                "reject_once".to_string(),
            )
            .await
            .expect("permission registration should succeed");

        let permission_event = receiver
            .recv()
            .await
            .expect("permission event should arrive");
        assert!(matches!(
            permission_event.payload,
            StreamEventPayload::PermissionRequested { request }
                if request.request_id == "req_1"
                    && request.summary == "read_text_file README.md"
        ));

        let resolved = store
            .resolve_permission("alice", &session.id, "req_1", PermissionDecision::Approve)
            .await
            .expect("permission resolution should succeed");
        assert_eq!(resolved.request_id, "req_1");
        assert_eq!(resolved.decision, PermissionDecision::Approve);
        assert_eq!(
            resolution.wait().await,
            PermissionResolutionOutcome::Selected("allow_once".to_string())
        );
    }

    #[tokio::test]
    async fn permission_requests_without_active_turns_are_cancelled() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "permission please".to_string())
            .await
            .expect("prompt submission should succeed");

        let resolution = pending
            .register_permission_request(
                "read_text_file README.md".to_string(),
                "allow_once".to_string(),
                "reject_once".to_string(),
            )
            .await
            .expect("permission registration should not fail");

        assert_eq!(
            resolution.wait().await,
            PermissionResolutionOutcome::Cancelled
        );
    }

    #[tokio::test]
    async fn permission_requests_for_non_active_prompts_are_cancelled() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let first = store
            .submit_prompt("alice", &session.id, "first".to_string())
            .await
            .expect("first prompt submission should succeed");
        let second = store
            .submit_prompt("alice", &session.id, "second".to_string())
            .await
            .expect("second prompt submission should succeed");
        let _cancel_rx = first
            .start_turn()
            .await
            .expect("starting the first turn should succeed");

        let resolution = second
            .register_permission_request(
                "read_text_file README.md".to_string(),
                "allow_once".to_string(),
                "reject_once".to_string(),
            )
            .await
            .expect("mismatched permission registrations should not fail");

        assert_eq!(
            resolution.wait().await,
            PermissionResolutionOutcome::Cancelled
        );
    }

    #[tokio::test]
    async fn cancelling_the_active_turn_cancels_pending_permissions() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "permission please".to_string())
            .await
            .expect("prompt submission should succeed");

        let _cancel_rx = pending
            .start_turn()
            .await
            .expect("starting the turn should succeed");
        let resolution = pending
            .register_permission_request(
                "read_text_file README.md".to_string(),
                "allow_once".to_string(),
                "reject_once".to_string(),
            )
            .await
            .expect("permission registration should succeed");

        assert!(
            store
                .cancel_active_turn("alice", &session.id)
                .await
                .expect("cancelling should succeed")
        );
        assert_eq!(
            resolution.wait().await,
            PermissionResolutionOutcome::Cancelled
        );
    }

    #[tokio::test]
    async fn closed_sessions_reject_permission_registration_resolution_and_cancellation() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "permission please".to_string())
            .await
            .expect("prompt submission should succeed");
        store
            .close_session("alice", &session.id)
            .await
            .expect("closing the session should succeed");

        let registration_error = pending
            .register_permission_request(
                "read_text_file README.md".to_string(),
                "allow_once".to_string(),
                "reject_once".to_string(),
            )
            .await
            .expect_err("closed sessions should reject permission registration");
        assert_eq!(registration_error, SessionStoreError::Closed);

        let resolution_error = store
            .resolve_permission("alice", &session.id, "req_1", PermissionDecision::Approve)
            .await
            .expect_err("closed sessions should reject permission resolution");
        assert_eq!(resolution_error, SessionStoreError::Closed);

        let cancel_error = store
            .cancel_active_turn("alice", &session.id)
            .await
            .expect_err("closed sessions should reject turn cancellation");
        assert_eq!(cancel_error, SessionStoreError::Closed);
    }

    #[tokio::test]
    async fn closing_sessions_cancel_active_turns_and_pending_permissions() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        let pending = store
            .submit_prompt("alice", &session.id, "permission please".to_string())
            .await
            .expect("prompt submission should succeed");

        let mut cancel_rx = pending
            .start_turn()
            .await
            .expect("starting the turn should succeed");
        let resolution = pending
            .register_permission_request(
                "read_text_file README.md".to_string(),
                "allow_once".to_string(),
                "reject_once".to_string(),
            )
            .await
            .expect("permission registration should succeed");

        store
            .close_session("alice", &session.id)
            .await
            .expect("closing the session should succeed");

        cancel_rx
            .changed()
            .await
            .expect("closing the session should cancel the active turn");
        assert!(*cancel_rx.borrow());
        assert_eq!(
            resolution.wait().await,
            PermissionResolutionOutcome::Cancelled
        );
    }

    #[tokio::test]
    async fn cancelling_without_an_active_turn_reports_false() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");

        assert!(
            !store
                .cancel_active_turn("alice", &session.id)
                .await
                .expect("idle cancellation should succeed")
        );
    }

    #[tokio::test]
    async fn closed_sessions_reject_new_prompts_and_second_closes() {
        let store = SessionStore::new(4);
        let session = store
            .create_session("alice")
            .await
            .expect("session creation should succeed");
        store
            .close_session("alice", &session.id)
            .await
            .expect("closing should succeed");

        let prompt_error = store
            .submit_prompt("alice", &session.id, "hello".to_string())
            .await
            .expect_err("closed sessions should reject prompts");
        assert_eq!(prompt_error, SessionStoreError::Closed);
        assert_eq!(prompt_error.message(), "session already closed");

        let close_error = store
            .close_session("alice", &session.id)
            .await
            .expect_err("closing twice should fail");
        assert_eq!(close_error, SessionStoreError::Closed);
    }
}
