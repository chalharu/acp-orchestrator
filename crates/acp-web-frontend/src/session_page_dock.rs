use leptos::prelude::*;

use crate::components::composer::{Composer, ComposerControls};
use crate::session_page_callbacks::SessionViewCallbacks;
use crate::session_page_composer_signals::SessionComposerSignals;

#[component]
pub(crate) fn SessionDock(
    composer: SessionComposerSignals,
    callbacks: SessionViewCallbacks,
    draft: RwSignal<String>,
) -> impl IntoView {
    view! {
        <div class="chat-dock">
            <Composer
                draft=draft
                on_submit=callbacks.submit
                controls=ComposerControls {
                    disabled: composer.disabled,
                    status_text: composer.status,
                    show_cancel: composer.cancel_visible,
                    cancel_disabled: composer.cancel_busy,
                    on_cancel: callbacks.cancel,
                }
                slash_visible=composer.slash_palette_visible
                slash_candidates=composer.slash_candidates
                slash_selected_index=composer.slash_selected_index
                slash_apply_selected=composer.slash_apply_selected
                on_slash_select_next=callbacks.slash.select_next
                on_slash_select_previous=callbacks.slash.select_previous
                on_slash_apply_selected=callbacks.slash.apply_selected
                on_slash_apply_index=callbacks.slash.apply_index
                on_slash_dismiss=callbacks.slash.dismiss
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::SessionDock;
    use crate::session_page_callbacks::session_view_callbacks;
    use crate::session_page_composer_signals::session_composer_signals;
    use crate::session_page_signals::session_signals;
    use leptos::prelude::*;

    #[test]
    fn session_dock_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let draft = signals.draft;
            let composer = session_composer_signals(signals, Signal::derive(|| false));
            let callbacks = session_view_callbacks("s1".to_string(), signals);

            let _ = view! {
                <SessionDock
                    composer=composer
                    callbacks=callbacks
                    draft=draft
                />
            };
        });
    }
}
