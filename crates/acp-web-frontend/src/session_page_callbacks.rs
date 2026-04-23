use leptos::prelude::*;

use crate::session_page_actions::{
    delete_session_callback, rename_session_callback, session_permission_callbacks,
    session_submit_callback, slash_palette_callbacks, SessionSlashCallbacks,
};
use crate::session_page_signals::SessionSignals;

#[derive(Clone, Copy)]
pub(crate) struct SessionViewCallbacks {
    pub(crate) submit: Callback<String>,
    pub(crate) approve: Callback<String>,
    pub(crate) deny: Callback<String>,
    pub(crate) cancel: Callback<()>,
    pub(crate) slash: SessionSlashCallbacks,
    pub(crate) rename_session: Callback<(String, String)>,
    pub(crate) delete_session: Callback<String>,
}

pub(crate) fn session_view_callbacks(
    session_id: String,
    signals: SessionSignals,
) -> SessionViewCallbacks {
    let (approve, deny, cancel) = session_permission_callbacks(session_id.clone(), signals);

    SessionViewCallbacks {
        submit: session_submit_callback(session_id.clone(), signals),
        approve,
        deny,
        cancel,
        slash: slash_palette_callbacks(signals),
        rename_session: rename_session_callback(signals),
        delete_session: delete_session_callback(session_id, signals),
    }
}

#[cfg(test)]
mod tests {
    use super::session_view_callbacks;
    use crate::session_page_signals::session_signals;
    use leptos::prelude::*;

    #[test]
    fn session_view_callbacks_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let _ = session_view_callbacks("session-1".to_string(), signals);
        });
    }
}
