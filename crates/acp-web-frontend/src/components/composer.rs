//! Composer (message input + submit) component.

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
            <ComposerInput disabled=disabled draft=draft on_submit=on_submit />
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
