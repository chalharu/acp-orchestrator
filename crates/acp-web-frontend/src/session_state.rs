use acp_contracts_permissions::PermissionRequest;

use crate::session_lifecycle::{SessionLifecycle, TurnState};

pub(crate) fn session_action_busy(
    turn_state: TurnState,
    pending_action_busy: bool,
    action_in_progress: bool,
) -> bool {
    pending_action_busy || action_in_progress || turn_state != TurnState::Idle
}

pub(crate) fn session_composer_disabled(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    current_session_deleting: bool,
) -> bool {
    current_session_deleting
        || session_status != SessionLifecycle::Active
        || turn_state != TurnState::Idle
}

pub(crate) fn session_composer_status_message(
    session_status: SessionLifecycle,
    turn_state: TurnState,
    current_session_deleting: bool,
) -> String {
    if current_session_deleting {
        return "Deleting session...".to_string();
    }
    match turn_state {
        TurnState::Submitting | TurnState::AwaitingReply => "Waiting for response...".to_string(),
        TurnState::AwaitingPermission => {
            "Resolve the pending request before sending another message.".to_string()
        }
        TurnState::Cancelling => "Cancelling...".to_string(),
        TurnState::Idle => match session_status {
            SessionLifecycle::Active => String::new(),
            SessionLifecycle::Closed => "This conversation has ended.".to_string(),
            SessionLifecycle::Loading => "Connecting...".to_string(),
            SessionLifecycle::Unavailable | SessionLifecycle::Error => {
                "Session unavailable. Start a fresh chat.".to_string()
            }
        },
    }
}

pub(crate) fn session_composer_cancel_visible(
    turn_state: TurnState,
    has_pending_permissions: bool,
    current_session_deleting: bool,
) -> bool {
    !current_session_deleting
        && !has_pending_permissions
        && matches!(turn_state, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn should_apply_snapshot_turn_state(current: TurnState) -> bool {
    matches!(current, TurnState::Idle | TurnState::AwaitingPermission)
}

pub(crate) fn should_release_turn_state(current: TurnState) -> bool {
    matches!(current, TurnState::AwaitingReply | TurnState::Cancelling)
}

pub(crate) fn turn_state_for_snapshot(pending_permissions: &[PermissionRequest]) -> TurnState {
    if pending_permissions.is_empty() {
        TurnState::Idle
    } else {
        TurnState::AwaitingPermission
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_permissions::PermissionRequest;

    use super::{
        SessionLifecycle, TurnState, session_action_busy, session_composer_cancel_visible,
        session_composer_disabled, session_composer_status_message,
        should_apply_snapshot_turn_state, should_release_turn_state, turn_state_for_snapshot,
    };

    fn assert_composer_status_cases(cases: &[(SessionLifecycle, TurnState, bool, &str)]) {
        for (session_status, turn_state, current_session_deleting, expected) in cases {
            assert_eq!(
                session_composer_status_message(
                    *session_status,
                    *turn_state,
                    *current_session_deleting,
                ),
                *expected
            );
        }
    }

    fn assert_cancel_visibility_cases(cases: &[(TurnState, bool, bool, bool)]) {
        for (turn_state, has_pending_permissions, current_session_deleting, expected) in cases {
            assert_eq!(
                session_composer_cancel_visible(
                    *turn_state,
                    *has_pending_permissions,
                    *current_session_deleting,
                ),
                *expected
            );
        }
    }

    #[test]
    fn session_composer_helpers_match_turn_state() {
        assert!(session_composer_disabled(
            SessionLifecycle::Active,
            TurnState::AwaitingReply,
            false,
        ));
        assert_eq!(
            session_composer_status_message(
                SessionLifecycle::Active,
                TurnState::AwaitingPermission,
                false,
            ),
            "Resolve the pending request before sending another message."
        );
        assert_eq!(
            session_composer_status_message(SessionLifecycle::Closed, TurnState::Idle, false),
            "This conversation has ended."
        );
        assert!(session_composer_cancel_visible(
            TurnState::AwaitingReply,
            false,
            false,
        ));
    }

    #[test]
    fn turn_state_helpers_match_permission_state() {
        assert!(should_release_turn_state(TurnState::AwaitingReply));
        assert_eq!(turn_state_for_snapshot(&[]), TurnState::Idle);
        assert_eq!(
            turn_state_for_snapshot(&[PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read README.md".to_string(),
            }]),
            TurnState::AwaitingPermission
        );
        assert!(session_action_busy(TurnState::Submitting, false, false));
    }

    #[test]
    fn composer_status_messages_cover_active_running_and_loading_states() {
        assert_composer_status_cases(&[
            (
                SessionLifecycle::Active,
                TurnState::Idle,
                true,
                "Deleting session...",
            ),
            (
                SessionLifecycle::Active,
                TurnState::Submitting,
                false,
                "Waiting for response...",
            ),
            (
                SessionLifecycle::Active,
                TurnState::AwaitingReply,
                false,
                "Waiting for response...",
            ),
            (
                SessionLifecycle::Active,
                TurnState::Cancelling,
                false,
                "Cancelling...",
            ),
            (SessionLifecycle::Active, TurnState::Idle, false, ""),
            (
                SessionLifecycle::Loading,
                TurnState::Idle,
                false,
                "Connecting...",
            ),
        ]);
    }

    #[test]
    fn composer_status_messages_cover_unavailable_states() {
        assert_composer_status_cases(&[
            (
                SessionLifecycle::Unavailable,
                TurnState::Idle,
                false,
                "Session unavailable. Start a fresh chat.",
            ),
            (
                SessionLifecycle::Error,
                TurnState::Idle,
                false,
                "Session unavailable. Start a fresh chat.",
            ),
        ]);
    }

    #[test]
    fn composer_cancel_visibility_hides_for_permissions_and_deletes() {
        assert_cancel_visibility_cases(&[
            (TurnState::AwaitingReply, true, false, false),
            (TurnState::AwaitingReply, false, true, false),
            (TurnState::Cancelling, false, false, true),
        ]);
    }

    #[test]
    fn turn_state_helpers_cover_snapshot_application_and_idle_paths() {
        assert!(should_apply_snapshot_turn_state(TurnState::Idle));
        assert!(should_apply_snapshot_turn_state(
            TurnState::AwaitingPermission
        ));
        assert!(!should_apply_snapshot_turn_state(TurnState::Submitting));
        assert!(!should_release_turn_state(TurnState::Idle));
        assert_eq!(turn_state_for_snapshot(&[]), TurnState::Idle);
        assert!(!session_action_busy(TurnState::Idle, false, false));
    }
}
