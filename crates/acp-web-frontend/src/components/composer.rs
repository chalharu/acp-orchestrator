//! Composer (message input + submit) component.

use acp_contracts::CompletionCandidate;
use leptos::prelude::*;

#[component]
pub fn Composer(
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] status_text: Signal<String>,
    draft: RwSignal<String>,
    on_submit: Callback<String>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
    #[prop(into)] slash_palette_visible: Signal<bool>,
    #[prop(into)] slash_candidates: Signal<Vec<CompletionCandidate>>,
    #[prop(into)] slash_selected_index: Signal<usize>,
    #[prop(into)] slash_loading: Signal<bool>,
    #[prop(into)] slash_error: Signal<Option<String>>,
    #[prop(into)] slash_apply_on_enter: Signal<bool>,
    on_slash_select_next: Callback<()>,
    on_slash_select_previous: Callback<()>,
    on_slash_apply_selected: Callback<()>,
    on_slash_apply_index: Callback<usize>,
    on_slash_dismiss: Callback<()>,
) -> impl IntoView {
    let handle_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let text = draft.get_untracked().trim().to_string();
        if text.is_empty() || disabled.get_untracked() {
            return;
        }
        on_submit.run(text);
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
                slash_palette_visible=slash_palette_visible
                slash_apply_on_enter=slash_apply_on_enter
                on_slash_select_next=on_slash_select_next
                on_slash_select_previous=on_slash_select_previous
                on_slash_apply_selected=on_slash_apply_selected
                on_slash_dismiss=on_slash_dismiss
            />
            <SlashPalette
                visible=slash_palette_visible
                candidates=slash_candidates
                selected_index=slash_selected_index
                loading=slash_loading
                error=slash_error
                on_apply_index=on_slash_apply_index
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
    #[prop(into)] slash_palette_visible: Signal<bool>,
    #[prop(into)] slash_apply_on_enter: Signal<bool>,
    on_slash_select_next: Callback<()>,
    on_slash_select_previous: Callback<()>,
    on_slash_apply_selected: Callback<()>,
    on_slash_dismiss: Callback<()>,
) -> impl IntoView {
    let handle_submit = move || {
        let text = draft.get_untracked().trim().to_string();
        if text.is_empty() || disabled.get_untracked() {
            return;
        }
        on_submit.run(text);
    };

    view! {
        <label class="sr-only" for="composer-input">"Prompt"</label>
        <textarea
            id="composer-input"
            name="prompt"
            rows="4"
            placeholder="Write a prompt or next step."
            prop:value=move || draft.get()
            on:input=move |ev| {
                let target = event_target::<web_sys::HtmlTextAreaElement>(&ev);
                draft.set(target.value());
            }
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                if slash_palette_visible.get_untracked() {
                    match ev.key().as_str() {
                        "ArrowDown" => {
                            ev.prevent_default();
                            on_slash_select_next.run(());
                            return;
                        }
                        "ArrowUp" => {
                            ev.prevent_default();
                            on_slash_select_previous.run(());
                            return;
                        }
                        "Tab" => {
                            ev.prevent_default();
                            on_slash_apply_selected.run(());
                            return;
                        }
                        "Enter" if !ev.shift_key() => {
                            ev.prevent_default();
                            if slash_apply_on_enter.get_untracked() {
                                on_slash_apply_selected.run(());
                            } else {
                                handle_submit();
                            }
                            return;
                        }
                        "Escape" => {
                            ev.prevent_default();
                            on_slash_dismiss.run(());
                            return;
                        }
                        _ => {}
                    }
                }
                if ev.is_composing() {
                    return;
                }
                if ev.key() == "Enter" && !ev.shift_key() {
                    ev.prevent_default();
                    handle_submit();
                }
            }
            prop:disabled=move || disabled.get()
        />
    }
}

#[component]
fn SlashPalette(
    #[prop(into)] visible: Signal<bool>,
    #[prop(into)] candidates: Signal<Vec<CompletionCandidate>>,
    #[prop(into)] selected_index: Signal<usize>,
    #[prop(into)] loading: Signal<bool>,
    #[prop(into)] error: Signal<Option<String>>,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    let palette_items =
        Signal::derive(move || candidates.get().into_iter().enumerate().collect::<Vec<_>>());

    view! {
        <Show when=move || visible.get()>
            <section class="composer__slash-palette" aria-label="Slash command suggestions">
                <Show
                    when=move || error.get().is_none()
                    fallback=move || {
                        view! {
                            <p class="composer__slash-empty composer__slash-empty--error">
                                {move || error.get().unwrap_or_default()}
                            </p>
                        }
                    }
                >
                    <Show
                        when=move || !loading.get()
                        fallback=move || {
                            view! { <p class="composer__slash-empty">"Looking up slash commands…"</p> }
                        }
                    >
                        <Show
                            when=move || !palette_items.get().is_empty()
                            fallback=move || {
                                view! { <p class="composer__slash-empty">"No matching slash commands."</p> }
                            }
                        >
                            <ul class="composer__slash-list">
                                <For
                                    each=move || palette_items.get()
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
                        </Show>
                    </Show>
                </Show>
            </section>
        </Show>
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
