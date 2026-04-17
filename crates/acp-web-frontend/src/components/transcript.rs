//! Conversation transcript component.

use leptos::prelude::*;

use crate::TranscriptEntry;

#[component]
pub fn Transcript(#[prop(into)] entries: Signal<Vec<TranscriptEntry>>) -> impl IntoView {
    view! {
        <Show when=move || !entries.get().is_empty()>
            <section aria-label="conversation transcript">
                <ol class="transcript">
                    <For
                        each=move || entries.get()
                        key=|entry| entry.id.clone()
                        children=move |entry| {
                            let css = entry.role.css_class();
                            let label = entry.role.label();
                            let text = entry.text;

                            view! {
                                <li class=format!("transcript-entry {css}")>
                                    <p class="transcript-entry__body">
                                        <span class="sr-only">{format!("{label}: ")}</span>
                                        {text}
                                    </p>
                                </li>
                            }
                        }
                    />
                </ol>
            </section>
        </Show>
    }
}
