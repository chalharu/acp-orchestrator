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
            <div class="section-heading section-heading--compact">
                <div>
                    <p class="eyebrow">"Next turn"</p>
                    <h2>"Write the next turn"</h2>
                </div>
                <p class="section-heading__meta">{move || status_text.get()}</p>
            </div>
            <label class="sr-only" for="composer-input">"Prompt"</label>
            <textarea
                id="composer-input"
                name="prompt"
                rows="4"
                placeholder="Write a prompt, question, or next step…"
                prop:value=move || draft.get()
                on:input=move |ev| {
                    let target = event_target::<web_sys::HtmlTextAreaElement>(&ev);
                    draft.set(target.value());
                }
                prop:disabled=move || busy.get()
            />
            <div class="composer__footer">
                <p class="muted">
                    "Short prompts work well, but longer notes are welcome too."
                </p>
                <button
                    type="submit"
                    prop:disabled=move || busy.get()
                >
                    "Send prompt"
                </button>
            </div>
        </form>
    }
}
