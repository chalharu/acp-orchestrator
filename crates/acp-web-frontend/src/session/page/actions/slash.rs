#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_slash::CompletionCandidate;
use leptos::prelude::*;

#[cfg(target_family = "wasm")]
use crate::browser::clear_draft;
#[cfg(target_family = "wasm")]
use crate::browser::clear_prepared_session_id;
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;
use crate::session_lifecycle::TurnState;
use crate::slash::{
    BrowserSlashAction, apply_slash_completion, cycle_slash_selection, local_browser_commands,
    local_slash_candidates, parse_browser_slash_action,
};

use super::super::state::{SessionSignals, SessionSlashCallbacks};
#[cfg(target_family = "wasm")]
use super::session_list::refresh_session_list;
#[cfg(target_family = "wasm")]
use super::shared::spawn_browser_task;
use super::stream::{next_tool_activity_id, push_tool_activity_entry};

fn update_slash_completion(signals: SessionSignals, draft: &str) {
    let candidates = local_slash_candidates(draft);
    if candidates.is_empty() {
        dismiss_slash_palette(signals);
    } else {
        signals.slash.candidates.set(candidates);
        signals.slash.selected_index.set(0);
    }
}

#[cfg(target_family = "wasm")]
pub(crate) fn bind_slash_completion(signals: SessionSignals) {
    Effect::new(move |_| {
        let draft = signals.draft.get();
        update_slash_completion(signals, &draft);
    });
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn bind_slash_completion(signals: SessionSignals) {
    update_slash_completion(signals, &signals.draft.get_untracked());
}

pub(crate) fn slash_palette_callbacks(signals: SessionSignals) -> SessionSlashCallbacks {
    SessionSlashCallbacks {
        select_next: Callback::new(move |()| {
            let next_index = cycle_slash_selection(
                signals.slash.candidates.get_untracked().len(),
                signals.slash.selected_index.get_untracked(),
                true,
            );
            signals.slash.selected_index.set(next_index);
        }),
        select_previous: Callback::new(move |()| {
            let next_index = cycle_slash_selection(
                signals.slash.candidates.get_untracked().len(),
                signals.slash.selected_index.get_untracked(),
                false,
            );
            signals.slash.selected_index.set(next_index);
        }),
        apply_selected: Callback::new(move |()| apply_selected_slash_candidate(signals)),
        apply_index: Callback::new(move |index: usize| apply_slash_candidate_at(signals, index)),
        dismiss: Callback::new(move |()| dismiss_slash_palette(signals)),
    }
}

#[cfg(target_family = "wasm")]
pub(crate) fn session_submit_callback(
    session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |prompt: String| {
        let session_id = session_id.clone();
        if prompt.starts_with('/') {
            handle_slash_submit(&prompt, signals);
            return;
        }

        signals.turn_state.set(TurnState::Submitting);
        signals.action_error.set(None);
        dismiss_slash_palette(signals);
        spawn_browser_task(async move {
            match api::send_message(&session_id, &prompt).await {
                Ok(()) => {
                    clear_prepared_session_id();
                    clear_draft(&session_id);
                    signals.draft.set(String::new());
                    signals.turn_state.set(TurnState::AwaitingReply);
                    refresh_session_list(signals).await;
                }
                Err(message) => {
                    signals.action_error.set(Some(message));
                    signals.turn_state.set(TurnState::Idle);
                }
            }
        });
    })
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn session_submit_callback(
    _session_id: String,
    signals: SessionSignals,
) -> Callback<String> {
    Callback::new(move |prompt: String| {
        if prompt.starts_with('/') {
            handle_slash_submit(&prompt, signals);
            return;
        }

        signals.turn_state.set(TurnState::Submitting);
        signals.action_error.set(None);
        dismiss_slash_palette(signals);
    })
}

fn apply_selected_slash_candidate(signals: SessionSignals) {
    let index = signals.slash.selected_index.get_untracked();
    apply_slash_candidate_at(signals, index);
}

fn apply_slash_candidate_at(signals: SessionSignals, index: usize) {
    let Some(candidate) = signals.slash.candidates.get_untracked().get(index).cloned() else {
        return;
    };
    let Some(next_draft) = apply_slash_completion(&signals.draft.get_untracked(), &candidate)
    else {
        return;
    };
    signals.draft.set(next_draft);
    signals.slash.selected_index.set(index);
}

fn dismiss_slash_palette(signals: SessionSignals) {
    signals.slash.candidates.set(Vec::new());
    signals.slash.selected_index.set(0);
}

fn handle_slash_submit(prompt: &str, signals: SessionSignals) {
    match parse_browser_slash_action(prompt) {
        Ok(action) => {
            signals.action_error.set(None);
            signals.draft.set(String::new());
            dismiss_slash_palette(signals);
            run_browser_slash_action(action, signals);
        }
        Err(message) => {
            push_tool_activity_entry(
                signals,
                next_tool_activity_id(signals, "slash"),
                "Slash command",
                message,
                Vec::new(),
            );
        }
    }
}

fn run_browser_slash_action(action: BrowserSlashAction, signals: SessionSignals) {
    match action {
        BrowserSlashAction::Help => {
            let commands = local_browser_commands();
            push_tool_activity_entry(
                signals,
                next_tool_activity_id(signals, "help"),
                "Available slash commands",
                available_slash_commands_detail(&commands),
                commands,
            );
        }
    }
}

fn available_slash_commands_detail(commands: &[CompletionCandidate]) -> String {
    if commands.is_empty() {
        "No browser slash commands are available.".to_string()
    } else {
        "Use the composer for `/help` and the on-screen controls for cancel or permission actions."
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_slash::{CompletionCandidate, CompletionKind};
    use leptos::prelude::*;

    use super::{
        apply_selected_slash_candidate, apply_slash_candidate_at, available_slash_commands_detail,
        bind_slash_completion, dismiss_slash_palette, handle_slash_submit, session_submit_callback,
        slash_palette_callbacks, update_slash_completion,
    };
    use crate::session::page::state::session_signals;
    use crate::session_lifecycle::TurnState;

    fn candidate(label: &str) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            insert_text: label.to_string(),
            detail: "detail".to_string(),
            kind: CompletionKind::Command,
        }
    }

    #[test]
    fn slash_completion_updates_or_dismisses_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            update_slash_completion(signals, "/");
            assert_eq!(signals.slash.candidates.get().len(), 1);
            assert_eq!(signals.slash.selected_index.get(), 0);

            update_slash_completion(signals, "hello");
            assert!(signals.slash.candidates.get().is_empty());
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn bind_slash_completion_uses_the_current_host_draft() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/".to_string());

            bind_slash_completion(signals);

            assert_eq!(signals.slash.candidates.get().len(), 1);
        });
    }

    #[test]
    fn slash_callbacks_cycle_apply_and_dismiss() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/he".to_string());
            signals
                .slash
                .candidates
                .set(vec![candidate("/help"), candidate("/quit")]);
            let callbacks = slash_palette_callbacks(signals);

            callbacks.select_next.run(());
            assert_eq!(signals.slash.selected_index.get(), 1);
            callbacks.select_previous.run(());
            assert_eq!(signals.slash.selected_index.get(), 0);

            callbacks.apply_index.run(0);
            assert_eq!(signals.draft.get(), "/help");

            signals.draft.set("/he".to_string());
            callbacks.apply_selected.run(());
            assert_eq!(signals.draft.get(), "/help");

            callbacks.dismiss.run(());
            assert!(signals.slash.candidates.get().is_empty());
        });
    }

    #[test]
    fn slash_candidate_helpers_skip_invalid_indexes() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/he".to_string());
            signals.slash.candidates.set(vec![candidate("/help")]);

            apply_slash_candidate_at(signals, 99);
            assert_eq!(signals.draft.get(), "/he");

            signals.slash.selected_index.set(0);
            apply_selected_slash_candidate(signals);
            assert_eq!(signals.draft.get(), "/help");

            dismiss_slash_palette(signals);
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn slash_submit_records_success_and_error_activity_messages() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/help".to_string());
            handle_slash_submit("/help", signals);
            assert!(signals.action_error.get().is_none());
            assert!(signals.draft.get().is_empty());
            assert_eq!(signals.entries.get().len(), 1);
            assert!(
                signals.entries.get()[0]
                    .text
                    .contains("Available slash commands")
            );

            handle_slash_submit("/quit", signals);
            assert_eq!(signals.entries.get().len(), 2);
            assert!(
                signals.entries.get()[1]
                    .text
                    .contains("Use the session list")
            );
        });
    }

    #[test]
    fn host_submit_callback_updates_turn_state_for_prompts() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.action_error.set(Some("old".to_string()));
            signals.slash.candidates.set(vec![candidate("/help")]);
            let submit = session_submit_callback("session-1".to_string(), signals);

            submit.run("hello".to_string());

            assert_eq!(signals.turn_state.get(), TurnState::Submitting);
            assert!(signals.action_error.get().is_none());
            assert!(signals.slash.candidates.get().is_empty());
        });
    }

    #[test]
    fn host_submit_callback_routes_slash_prompts_to_local_actions() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            let submit = session_submit_callback("session-1".to_string(), signals);

            submit.run("/help".to_string());

            assert_eq!(signals.turn_state.get(), TurnState::Idle);
            assert_eq!(signals.entries.get().len(), 1);
            assert!(
                signals.entries.get()[0]
                    .text
                    .contains("Available slash commands")
            );
        });
    }

    #[test]
    fn slash_candidate_application_skips_completions_that_do_not_change_the_draft() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("/help".to_string());
            signals.slash.candidates.set(vec![candidate("/help")]);

            apply_slash_candidate_at(signals, 0);

            assert_eq!(signals.draft.get(), "/help");
            assert_eq!(signals.slash.selected_index.get(), 0);
        });
    }

    #[test]
    fn slash_candidate_application_ignores_drafts_without_a_completion_prefix() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = session_signals();
            signals.draft.set("hello".to_string());
            signals.slash.candidates.set(vec![candidate("/help")]);

            apply_slash_candidate_at(signals, 0);

            assert_eq!(signals.draft.get(), "hello");
        });
    }

    #[test]
    fn available_slash_commands_detail_mentions_palette_and_controls() {
        assert_eq!(
            available_slash_commands_detail(&[]),
            "No browser slash commands are available."
        );
        assert!(available_slash_commands_detail(&[candidate("/help")]).contains("composer"));
    }
}
