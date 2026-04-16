//! Composer (message input + submit) component.

use leptos::prelude::*;

#[component]
pub fn Composer(
    #[prop(into)] busy: Signal<bool>,
    #[prop(into)] status_text: Signal<String>,
    on_submit: Callback<String>,
) -> impl IntoView {
    let input_value = RwSignal::new(String::new());

    let handle_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let text = input_value.get_untracked().trim().to_string();
        if text.is_empty() || busy.get_untracked() {
            return;
        }
        input_value.set(String::new());
        on_submit.run(text);
    };

    view! {
        <form
            class="panel composer"
            autocomplete="off"
            on:submit=handle_submit
        >
            <label for="composer-input">"Prompt"</label>
            <textarea
                id="composer-input"
                name="prompt"
                rows="4"
                placeholder="Ask ACP something…"
                prop:value=move || input_value.get()
                on:input=move |ev| {
                    let target = event_target::<web_sys::HtmlTextAreaElement>(&ev);
                    input_value.set(target.value());
                }
                prop:disabled=move || busy.get()
            />
            <div class="composer__footer">
                <p class="muted">{move || status_text.get()}</p>
                <button
                    type="submit"
                    prop:disabled=move || busy.get()
                >
                    "Send"
                </button>
            </div>
        </form>
    }
}
