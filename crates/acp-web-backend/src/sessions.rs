use std::{collections::HashMap, sync::Arc};

use acp_contracts::{
    ConversationMessage, PermissionDecision, ResolvePermissionResponse, SessionSnapshot,
    StreamEvent,
};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot, watch};
use uuid::Uuid;

mod handle;

#[cfg(test)]
mod tests;

use handle::{PromptCompletion, SessionHandle};

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
pub struct TurnHandle {
    handle: Arc<SessionHandle>,
    session_id: String,
    prompt_text: String,
    prompt_order: u64,
}

impl TurnHandle {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn prompt_text(&self) -> &str {
        &self.prompt_text
    }

    pub(crate) async fn start_turn(&self) -> Result<watch::Receiver<bool>, SessionStoreError> {
        self.handle.start_turn(self.prompt_order).await
    }

    pub(crate) async fn is_active(&self) -> bool {
        self.handle.is_active().await
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
}

#[derive(Debug, Clone)]
pub struct PendingPrompt {
    turn: TurnHandle,
}

impl PendingPrompt {
    pub fn session_id(&self) -> &str {
        self.turn.session_id()
    }

    pub fn prompt_text(&self) -> &str {
        self.turn.prompt_text()
    }

    pub(crate) fn turn_handle(&self) -> TurnHandle {
        self.turn.clone()
    }

    pub async fn complete_with_reply(self, text: String) {
        if let Ok(events) = self
            .turn
            .handle
            .complete_prompt(self.turn.prompt_order, PromptCompletion::Reply(text))
            .await
        {
            for event in events {
                self.turn.handle.broadcast(event);
            }
        }
    }

    pub async fn complete_with_status(self, message: impl Into<String>) {
        if let Ok(events) = self
            .turn
            .handle
            .complete_prompt(
                self.turn.prompt_order,
                PromptCompletion::Status(message.into()),
            )
            .await
        {
            for event in events {
                self.turn.handle.broadcast(event);
            }
        }
    }

    pub async fn complete_without_output(self) {
        if let Ok(events) = self
            .turn
            .handle
            .complete_prompt(self.turn.prompt_order, PromptCompletion::None)
            .await
        {
            for event in events {
                self.turn.handle.broadcast(event);
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
            turn: TurnHandle {
                handle,
                session_id: session_id.to_string(),
                prompt_text: text,
                prompt_order,
            },
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
