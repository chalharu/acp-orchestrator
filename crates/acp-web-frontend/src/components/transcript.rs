//! Conversation transcript component.

use leptos::prelude::*;

use crate::TranscriptEntry;

#[component]
pub fn Transcript(#[prop(into)] entries: Signal<Vec<TranscriptEntry>>) -> impl IntoView {
    view! {
        <section class="panel transcript-panel" aria-label="conversation transcript">
            <ol class="transcript">
                <Show
                    when=move || !entries.get().is_empty()
                    fallback=move || {
                        view! {
                            <li class="transcript-entry transcript-entry--empty">
                                "No messages yet. Send your first prompt below."
                            </li>
                        }
                    }
                >
                    <For
                        each=move || entries.get()
                        key=|entry| entry.id.clone()
                        children=move |entry| {
                            let css = entry.role.css_class();
                            let label = entry.role.label();
                            let text = entry.text;
                            let id = entry.id;

                            view! {
                                <li class=format!("transcript-entry {css}")>
                                    <div class="transcript-entry__meta">
                                        <span>{label}</span>
                                        <span class="entry-id">{id}</span>
                                    </div>
                                    <p class="transcript-entry__body">{text}</p>
                                </li>
                            }
                        }
                    />
                </Show>
            </ol>
        </section>
    }
}
