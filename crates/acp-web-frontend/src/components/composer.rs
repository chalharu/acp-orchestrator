//! Composer (message input + submit) component.

use acp_contracts::CompletionCandidate;
use leptos::prelude::*;

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
    Loading,
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
    let handle_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        submit_draft(draft, disabled, on_submit);
    };

    view! {
        <form
            class="panel composer"
            autocomplete="off"
            on:submit=handle_submit
        >
            <ComposerInput
                disabled=disabled
                draft=draft
                on_submit=on_submit
                slash_signals=slash_signals
                slash_callbacks=slash_callbacks
            />
            <SlashPalette
                slash_signals=slash_signals
                on_apply_index=slash_callbacks.apply_index
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
fn ComposerInput(
    #[prop(into)] disabled: Signal<bool>,
    draft: RwSignal<String>,
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
            placeholder="Write a prompt or type / for commands."
            prop:value=move || draft.get()
            on:input=move |ev| update_draft(draft, &ev)
            on:keydown=move |ev| {
                handle_composer_keydown(
                    ev,
                    draft,
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

fn update_draft(draft: RwSignal<String>, ev: &web_sys::Event) {
    let target = event_target::<web_sys::HtmlTextAreaElement>(ev);
    draft.set(target.value());
}

fn handle_composer_keydown(
    ev: web_sys::KeyboardEvent,
    draft: RwSignal<String>,
    disabled: Signal<bool>,
    on_submit: Callback<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) {
    if handle_slash_palette_keydown(
        &ev,
        draft,
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
        submit_draft(draft, disabled, on_submit);
    }
}

fn handle_slash_palette_keydown(
    ev: &web_sys::KeyboardEvent,
    draft: RwSignal<String>,
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
                submit_draft(draft, disabled, on_submit);
            }
        }
        "Escape" => slash_callbacks.dismiss.run(()),
        _ => return false,
    }

    ev.prevent_default();
    true
}

fn submit_draft(draft: RwSignal<String>, disabled: Signal<bool>, on_submit: Callback<String>) {
    let text = draft.get_untracked().trim().to_string();
    if text.is_empty() || disabled.get_untracked() {
        return;
    }
    on_submit.run(text);
}

#[component]
fn SlashPalette(
    slash_signals: ComposerSlashSignals,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    let selected_index = slash_signals.selected_index;
    let state = Signal::derive(move || slash_palette_state(slash_signals));

    view! {
        <Show when=move || slash_signals.visible.get()>
            <section class="composer__slash-palette" aria-label="Slash command suggestions">
                {move || render_slash_palette_state(state.get(), selected_index, on_apply_index)}
            </section>
        </Show>
    }
}

fn slash_palette_state(slash_signals: ComposerSlashSignals) -> SlashPaletteState {
    if let Some(message) = slash_signals.error.get() {
        return SlashPaletteState::Error(message);
    }

    if slash_signals.loading.get() {
        return SlashPaletteState::Loading;
    }

    let items = slash_signals
        .candidates
        .get()
        .into_iter()
        .enumerate()
        .collect::<Vec<_>>();
    if items.is_empty() {
        SlashPaletteState::Empty
    } else {
        SlashPaletteState::Ready(items)
    }
}

fn render_slash_palette_state(
    state: SlashPaletteState,
    selected_index: Signal<usize>,
    on_apply_index: Callback<usize>,
) -> AnyView {
    match state {
        SlashPaletteState::Error(message) => view! {
            <p class="composer__slash-empty composer__slash-empty--error">
                {message}
            </p>
        }
        .into_any(),
        SlashPaletteState::Loading => {
            view! { <p class="composer__slash-empty">"Looking up slash commands…"</p> }.into_any()
        }
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
    #[prop(into)] selected_index: Signal<usize>,
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
                            is_selected=Signal::derive(move || selected_index.get() == index)
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
    #[prop(into)] is_selected: Signal<bool>,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    let CompletionCandidate { label, detail, .. } = candidate;

    view! {
        <li>
            <button
                type="button"
                class=move || {
                    if is_selected.get() {
                        "composer__slash-item composer__slash-item--selected"
                    } else {
                        "composer__slash-item"
                    }
                }
                on:click=move |_| on_apply_index.run(index)
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
