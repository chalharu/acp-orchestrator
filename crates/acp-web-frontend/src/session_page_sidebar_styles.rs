pub(super) fn session_sidebar_class(sidebar_open: bool) -> &'static str {
    if sidebar_open {
        "session-sidebar session-sidebar--open"
    } else {
        "session-sidebar"
    }
}

pub(super) fn session_sidebar_item_class(is_current: bool, is_closed: bool) -> &'static str {
    match (is_current, is_closed) {
        (true, true) => {
            "session-sidebar__item session-sidebar__item--current session-sidebar__item--closed"
        }
        (true, false) => "session-sidebar__item session-sidebar__item--current",
        (false, true) => "session-sidebar__item session-sidebar__item--closed",
        (false, false) => "session-sidebar__item",
    }
}

pub(super) fn session_sidebar_empty_message(has_error: bool) -> &'static str {
    if has_error {
        "Unable to load sessions right now."
    } else {
        "No sessions yet. Start a new one."
    }
}

pub(super) fn session_sidebar_status_label(is_closed: bool) -> &'static str {
    if is_closed {
        "closed"
    } else {
        "active"
    }
}

pub(super) fn session_sidebar_status_pill_class(is_closed: bool) -> &'static str {
    if is_closed {
        "session-sidebar__status-pill session-sidebar__status-pill--neutral"
    } else {
        "session-sidebar__status-pill session-sidebar__status-pill--success"
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn sidebar_delete_sr_label(is_deleting: bool) -> &'static str {
    if is_deleting {
        "Deleting…"
    } else {
        "Delete session"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_sidebar_class_adds_open_modifier_when_sidebar_is_open() {
        assert_eq!(
            session_sidebar_class(true),
            "session-sidebar session-sidebar--open"
        );
        assert_eq!(session_sidebar_class(false), "session-sidebar");
    }

    #[test]
    fn session_sidebar_item_class_applies_current_and_closed_modifiers() {
        let both = session_sidebar_item_class(true, true);
        assert!(both.contains("--current"));
        assert!(both.contains("--closed"));

        let current_only = session_sidebar_item_class(true, false);
        assert!(current_only.contains("--current"));
        assert!(!current_only.contains("--closed"));

        let closed_only = session_sidebar_item_class(false, true);
        assert!(!closed_only.contains("--current"));
        assert!(closed_only.contains("--closed"));

        assert_eq!(
            session_sidebar_item_class(false, false),
            "session-sidebar__item"
        );
    }

    #[test]
    fn session_sidebar_empty_message_differs_based_on_error_presence() {
        assert!(session_sidebar_empty_message(true).contains("Unable to load"));
        assert!(session_sidebar_empty_message(false).contains("No sessions yet"));
    }

    #[test]
    fn delete_labels_match_sidebar_state() {
        assert_eq!(sidebar_delete_sr_label(true), "Deleting…");
        assert_eq!(sidebar_delete_sr_label(false), "Delete session");
    }

    #[test]
    fn status_labels_and_pills_match_closed_state() {
        assert_eq!(session_sidebar_status_label(false), "active");
        assert_eq!(session_sidebar_status_label(true), "closed");
        assert_eq!(
            session_sidebar_status_pill_class(false),
            "session-sidebar__status-pill session-sidebar__status-pill--success"
        );
        assert_eq!(
            session_sidebar_status_pill_class(true),
            "session-sidebar__status-pill session-sidebar__status-pill--neutral"
        );
    }
}
