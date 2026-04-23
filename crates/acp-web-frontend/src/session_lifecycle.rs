#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_sessions::SessionStatus;

pub(crate) const CLOSED_SESSION_MESSAGE: &str = "Conversation ended.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BadgeTone {
    Neutral,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionLifecycle {
    Loading,
    Active,
    Closed,
    Unavailable,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnState {
    Idle,
    Submitting,
    AwaitingReply,
    AwaitingPermission,
    Cancelling,
}

pub(crate) fn session_end_message(reason: Option<&str>) -> String {
    let Some(reason) = reason.map(str::trim) else {
        return CLOSED_SESSION_MESSAGE.to_string();
    };
    if reason.is_empty() || reason == "closed by user" {
        CLOSED_SESSION_MESSAGE.to_string()
    } else {
        reason.to_string()
    }
}

pub(crate) fn session_status_label(status: SessionStatus) -> SessionLifecycle {
    match status {
        SessionStatus::Active => SessionLifecycle::Active,
        SessionStatus::Closed => SessionLifecycle::Closed,
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_sessions::SessionStatus;

    use super::{
        CLOSED_SESSION_MESSAGE, SessionLifecycle, session_end_message, session_status_label,
    };

    #[test]
    fn session_end_message_normalizes_empty_and_default_reasons() {
        assert_eq!(session_end_message(None), CLOSED_SESSION_MESSAGE);
        assert_eq!(session_end_message(Some("   ")), CLOSED_SESSION_MESSAGE);
        assert_eq!(
            session_end_message(Some(" closed by user ")),
            CLOSED_SESSION_MESSAGE
        );
        assert_eq!(session_end_message(Some(" timeout ")), "timeout");
    }

    #[test]
    fn session_status_label_maps_backend_statuses() {
        assert_eq!(
            session_status_label(SessionStatus::Active),
            SessionLifecycle::Active
        );
        assert_eq!(
            session_status_label(SessionStatus::Closed),
            SessionLifecycle::Closed
        );
    }
}
