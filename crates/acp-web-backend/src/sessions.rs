use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use acp_contracts::{
    ConversationMessage, MessageRole, SessionSnapshot, SessionStatus, StreamEvent,
    StreamEventPayload,
};
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, RwLock, broadcast};
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
    SessionCapReached,
}

impl SessionStoreError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotFound => "session not found",
            Self::Forbidden => "session owner mismatch",
            Self::Closed => "session already closed",
            Self::EmptyPrompt => "prompt must not be empty",
            Self::SessionCapReached => "session cap reached for principal",
        }
    }
}

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
enum PromptCompletion {
    Reply(String),
    Status(String),
}

#[derive(Debug)]
struct SessionData {
    id: String,
    owner: String,
    status: SessionStatus,
    closed_at: Option<DateTime<Utc>>,
    latest_sequence: u64,
    next_prompt_order: u64,
    next_completion_order: u64,
    pending_completions: BTreeMap<u64, PromptCompletion>,
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
                next_completion_order: 0,
                pending_completions: BTreeMap::new(),
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

    async fn complete_prompt(
        &self,
        prompt_order: u64,
        completion: PromptCompletion,
    ) -> Result<Vec<StreamEvent>, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }

        data.pending_completions.insert(prompt_order, completion);
        let mut events = Vec::new();
        loop {
            let next_completion_order = data.next_completion_order;
            let Some(completion) = data.pending_completions.remove(&next_completion_order) else {
                break;
            };
            data.next_completion_order += 1;
            events.push(match completion {
                PromptCompletion::Reply(text) => {
                    Self::message_event(&mut data, MessageRole::Assistant, text)
                }
                PromptCompletion::Status(message) => Self::status_event(&mut data, message),
            });
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
            .err()
            .expect("empty prompt should fail");

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
            .err()
            .expect("closed sessions should reject prompts");
        assert_eq!(prompt_error, SessionStoreError::Closed);
        assert_eq!(prompt_error.message(), "session already closed");

        let close_error = store
            .close_session("alice", &session.id)
            .await
            .expect_err("closing twice should fail");
        assert_eq!(close_error, SessionStoreError::Closed);
    }
}
