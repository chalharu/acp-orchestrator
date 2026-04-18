//! Composer (message input + submit) component.

use acp_contracts::CompletionCandidate;
use leptos::{html as leptos_html, prelude::*};

const MAX_SLASH_PALETTE_ITEMS: usize = 5;

#[derive(Clone, Copy)]
pub struct ComposerSlashSignals {
    pub visible: Signal<bool>,
    pub candidates: Signal<Vec<CompletionCandidate>>,
    pub selected_index: Signal<usize>,
    pub loading: Signal<bool>,
    pub error: Signal<Option<String>>,
    pub apply_on_enter: Signal<bool>,
}

#[derive(Clone, Copy)]
pub struct ComposerSlashCallbacks {
    pub select_next: Callback<()>,
    pub select_previous: Callback<()>,
    pub apply_selected: Callback<()>,
    pub apply_index: Callback<usize>,
    pub dismiss: Callback<()>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlashPaletteState {
    Error(String),
    Empty,
    Ready(Vec<(usize, CompletionCandidate)>),
}

#[component]
pub fn Composer(
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] status_text: Signal<String>,
    draft: RwSignal<String>,
    on_submit: Callback<String>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) -> impl IntoView {
    let textarea = NodeRef::<leptos_html::Textarea>::new();
    let handle_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        submit_draft(draft, textarea, disabled, on_submit);
    };

    view! {
        <form
            class="panel composer"
            autocomplete="off"
            on:submit=handle_submit
        >
            <ComposerEditor
                disabled=disabled
                draft=draft
                textarea=textarea
                on_submit=on_submit
                slash_signals=slash_signals
                slash_callbacks=slash_callbacks
            />
            <ComposerFooter
                status_text=status_text
                disabled=disabled
                show_cancel=show_cancel
                cancel_disabled=cancel_disabled
                on_cancel=on_cancel
            />
        </form>
    }
}

#[component]
fn ComposerEditor(
    #[prop(into)] disabled: Signal<bool>,
    draft: RwSignal<String>,
    textarea: NodeRef<leptos_html::Textarea>,
    on_submit: Callback<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) -> impl IntoView {
    view! {
        <div class="composer__editor">
            <ComposerInput
                disabled=disabled
                draft=draft
                textarea=textarea
                on_submit=on_submit
                slash_signals=slash_signals
                slash_callbacks=slash_callbacks
            />
            <SlashPalette
                slash_signals=slash_signals
                on_apply_index=slash_callbacks.apply_index
            />
        </div>
    }
}

#[component]
fn ComposerInput(
    #[prop(into)] disabled: Signal<bool>,
    draft: RwSignal<String>,
    textarea: NodeRef<leptos_html::Textarea>,
    on_submit: Callback<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) -> impl IntoView {
    view! {
        <label class="sr-only" for="composer-input">"Prompt"</label>
        <textarea
            id="composer-input"
            name="prompt"
            rows="4"
            node_ref=textarea
            placeholder="Write a prompt or type / for commands."
            prop:value=move || draft.get()
            on:input=move |ev| update_draft(draft, ev)
            on:keydown=move |ev| {
                handle_composer_keydown(
                    ev,
                    draft,
                    textarea,
                    disabled,
                    on_submit,
                    slash_signals,
                    slash_callbacks,
                );
            }
            prop:disabled=move || disabled.get()
        />
    }
}

fn update_draft(draft: RwSignal<String>, ev: web_sys::Event) {
    draft.set(event_target_value(&ev));
}

fn handle_composer_keydown(
    ev: web_sys::KeyboardEvent,
    draft: RwSignal<String>,
    textarea: NodeRef<leptos_html::Textarea>,
    disabled: Signal<bool>,
    on_submit: Callback<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) {
    if handle_slash_palette_keydown(
        &ev,
        draft,
        textarea,
        disabled,
        on_submit,
        slash_signals,
        slash_callbacks,
    ) || ev.is_composing()
    {
        return;
    }

    if ev.key() == "Enter" && !ev.shift_key() {
        ev.prevent_default();
        submit_draft(draft, textarea, disabled, on_submit);
    }
}

fn handle_slash_palette_keydown(
    ev: &web_sys::KeyboardEvent,
    draft: RwSignal<String>,
    textarea: NodeRef<leptos_html::Textarea>,
    disabled: Signal<bool>,
    on_submit: Callback<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) -> bool {
    if !slash_signals.visible.get_untracked() {
        return false;
    }

    match ev.key().as_str() {
        "ArrowDown" => slash_callbacks.select_next.run(()),
        "ArrowUp" => slash_callbacks.select_previous.run(()),
        "Tab" => slash_callbacks.apply_selected.run(()),
        "Enter" if !ev.shift_key() => {
            if slash_signals.apply_on_enter.get_untracked() {
                slash_callbacks.apply_selected.run(());
            } else {
                submit_draft(draft, textarea, disabled, on_submit);
            }
        }
        "Escape" => slash_callbacks.dismiss.run(()),
        _ => return false,
    }

    ev.prevent_default();
    true
}

fn submit_draft(
    draft: RwSignal<String>,
    textarea: NodeRef<leptos_html::Textarea>,
    disabled: Signal<bool>,
    on_submit: Callback<String>,
) {
    let signal_value = draft.get_untracked();
    let current_value = current_submit_value(
        textarea.get().map(|textarea| textarea.value()),
        signal_value.clone(),
    );

    if current_value != signal_value {
        draft.set(current_value.clone());
    }

    let Some(text) = submit_text(current_value, disabled.get_untracked()) else {
        return;
    };
    on_submit.run(text);
}

fn current_submit_value(live_value: Option<String>, draft_value: String) -> String {
    live_value.unwrap_or(draft_value)
}

fn submit_text(current_value: String, disabled: bool) -> Option<String> {
    let text = current_value.trim().to_string();
    (!disabled && !text.is_empty()).then_some(text)
}

#[component]
fn SlashPalette(
    slash_signals: ComposerSlashSignals,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    view! {
        <Show when=move || should_render_slash_palette(slash_signals)>
            <section class="composer__slash-palette" aria-label="Slash command suggestions">
                {move || {
                    render_slash_palette_state(
                        slash_palette_state(slash_signals),
                        slash_signals.selected_index.get(),
                        on_apply_index,
                    )
                }}
            </section>
        </Show>
    }
}

fn should_render_slash_palette(slash_signals: ComposerSlashSignals) -> bool {
    if !slash_signals.visible.get() {
        return false;
    }

    slash_signals.error.get().is_some()
        || !slash_signals.candidates.get().is_empty()
        || !slash_signals.loading.get()
}

fn slash_palette_state(slash_signals: ComposerSlashSignals) -> SlashPaletteState {
    if let Some(message) = slash_signals.error.get() {
        return SlashPaletteState::Error(message);
    }

    let items = slash_signals
        .candidates
        .get()
        .into_iter()
        .enumerate()
        .take(MAX_SLASH_PALETTE_ITEMS)
        .collect::<Vec<_>>();
    if items.is_empty() {
        SlashPaletteState::Empty
    } else {
        SlashPaletteState::Ready(items)
    }
}

fn render_slash_palette_state(
    state: SlashPaletteState,
    selected_index: usize,
    on_apply_index: Callback<usize>,
) -> AnyView {
    match state {
        SlashPaletteState::Error(message) => view! {
            <p class="composer__slash-empty composer__slash-empty--error">
                {message}
            </p>
        }
        .into_any(),
        SlashPaletteState::Empty => {
            view! { <p class="composer__slash-empty">"No matching slash commands."</p> }.into_any()
        }
        SlashPaletteState::Ready(items) => view! {
            <SlashPaletteList
                items=items
                selected_index=selected_index
                on_apply_index=on_apply_index
            />
        }
        .into_any(),
    }
}

#[component]
fn SlashPaletteList(
    items: Vec<(usize, CompletionCandidate)>,
    selected_index: usize,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    view! {
        <ul class="composer__slash-list">
            <For
                each=move || items.clone()
                key=|(index, candidate)| (index.to_owned(), candidate.label.clone())
                children=move |(index, candidate)| {
                    view! {
                        <SlashPaletteItem
                            index=index
                            candidate=candidate
                            is_selected=index == selected_index
                            on_apply_index=on_apply_index
                        />
                    }
                }
            />
        </ul>
    }
}

#[component]
fn SlashPaletteItem(
    index: usize,
    candidate: CompletionCandidate,
    is_selected: bool,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    let CompletionCandidate { label, detail, .. } = candidate;

    view! {
        <li>
            <button
                type="button"
                class=if is_selected {
                    "composer__slash-item composer__slash-item--selected"
                } else {
                    "composer__slash-item"
                }
                on:mousedown=move |ev| {
                    ev.prevent_default();
                    on_apply_index.run(index);
                }
                on:keydown=move |ev| {
                    if matches!(ev.key().as_str(), "Enter" | " ") {
                        ev.prevent_default();
                        on_apply_index.run(index);
                    }
                }
            >
                <span class="composer__slash-label">{label}</span>
                <span class="composer__slash-detail">{detail}</span>
            </button>
        </li>
    }
}

#[component]
fn ComposerFooter(
    #[prop(into)] status_text: Signal<String>,
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="composer__footer">
            <p class="composer__status" hidden=move || status_text.get().is_empty()>
                {move || status_text.get()}
            </p>
            <ComposerActions
                disabled=disabled
                show_cancel=show_cancel
                cancel_disabled=cancel_disabled
                on_cancel=on_cancel
            />
        </div>
    }
}

#[component]
fn ComposerActions(
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="composer__actions">
            <Show when=move || show_cancel.get()>
                <button
                    class="pending-list__button--secondary composer__cancel"
                    type="button"
                    on:click=move |_| on_cancel.run(())
                    prop:disabled=move || cancel_disabled.get()
                >
                    "Cancel"
                </button>
            </Show>
            <button
                class="composer__submit"
                type="submit"
                prop:disabled=move || disabled.get()
            >
                "Send"
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{current_submit_value, submit_text};

    #[test]
    fn current_submit_value_prefers_the_live_textarea_value() {
        assert_eq!(
            current_submit_value(Some("test".to_string()), "/help".to_string()),
            "test"
        );
    }

    #[test]
    fn submit_text_trims_and_blocks_disabled_or_empty_submissions() {
        assert_eq!(
            submit_text("  test  ".to_string(), false),
            Some("test".to_string())
        );
        assert_eq!(submit_text("   ".to_string(), false), None);
        assert_eq!(submit_text("test".to_string(), true), None);
    }
}
