use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use tokio::{
    sync::{Mutex, RwLock, broadcast},
    time::sleep,
};
use uuid::Uuid;

use crate::{
    mock_engine,
    models::{
        ConversationMessage, MessageRole, SessionSnapshot, SessionStatus, StreamEvent,
        StreamEventPayload,
    },
};

#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions: Arc<RwLock<HashMap<String, Arc<SessionHandle>>>>,
    create_session_lock: Arc<Mutex<()>>,
    closed_session_limit: usize,
    session_cap: usize,
    assistant_delay: Duration,
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

impl SessionStore {
    pub fn new(session_cap: usize, assistant_delay: Duration) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            create_session_lock: Arc::new(Mutex::new(())),
            closed_session_limit: 32,
            session_cap,
            assistant_delay,
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
    ) -> Result<(), SessionStoreError> {
        if text.trim().is_empty() {
            return Err(SessionStoreError::EmptyPrompt);
        }

        let handle = self.authorized_handle(owner, session_id).await?;
        let user_event = handle
            .append_message(MessageRole::User, text.clone())
            .await?;
        handle.broadcast(user_event);

        let handle_for_assistant = handle.clone();
        let assistant_delay = self.assistant_delay;
        tokio::spawn(async move {
            sleep(assistant_delay).await;
            if let Ok(event) = handle_for_assistant
                .append_message(MessageRole::Assistant, mock_engine::reply_for(&text))
                .await
            {
                handle_for_assistant.broadcast(event);
            }
        });

        Ok(())
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
struct SessionData {
    id: String,
    owner: String,
    status: SessionStatus,
    closed_at: Option<DateTime<Utc>>,
    latest_sequence: u64,
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

    async fn append_message(
        &self,
        role: MessageRole,
        text: String,
    ) -> Result<StreamEvent, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }

        data.latest_sequence += 1;
        let message = ConversationMessage {
            id: format!("m_{}", Uuid::new_v4().simple()),
            role,
            text,
            created_at: Utc::now(),
        };
        data.messages.push(message.clone());

        Ok(StreamEvent {
            sequence: data.latest_sequence,
            payload: StreamEventPayload::ConversationMessage { message },
        })
    }

    async fn close(&self, reason: &str) -> Result<StreamEvent, SessionStoreError> {
        let mut data = self.data.lock().await;
        if data.status == SessionStatus::Closed {
            return Err(SessionStoreError::Closed);
        }

        data.status = SessionStatus::Closed;
        data.closed_at = Some(Utc::now());
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
