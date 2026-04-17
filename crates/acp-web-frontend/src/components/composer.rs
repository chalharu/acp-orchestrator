//! Composer (message input + submit) component.

use leptos::prelude::*;

#[component]
pub fn Composer(
    #[prop(into)] busy: Signal<bool>,
    #[prop(into)] status_text: Signal<String>,
    draft: RwSignal<String>,
    on_submit: Callback<String>,
) -> impl IntoView {
    let handle_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let text = draft.get_untracked().trim().to_string();
        if text.is_empty() || busy.get_untracked() {
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
                prop:disabled=move || busy.get()
            />
            <div class="composer__footer">
                <p class="composer__status" hidden=move || status_text.get().is_empty()>
                    {move || status_text.get()}
                </p>
                <button
                    class="composer__submit"
                    type="submit"
                    prop:disabled=move || busy.get()
                >
                    "Send"
                </button>
            </div>
        </form>
    }
}
