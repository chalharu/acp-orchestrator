//! Pending permission panel (slice 1 read-only view).

use leptos::prelude::*;

/// Displays pending tool-permission requests.
///
/// Each item is `(request_id, summary)` sourced from
/// `acp_contracts::PermissionRequest`.
#[component]
pub fn PendingPermissions(#[prop(into)] items: Signal<Vec<(String, String)>>) -> impl IntoView {
    view! {
        <Show when=move || !items.get().is_empty()>
            <section
                class="panel pending-panel"
                aria-live="polite"
            >
                <h2>"Pending permissions"</h2>
                <p class="muted">
                    "Slice 1 renders pending requests and transcript updates. \
                     Approve/deny controls arrive in the next slice."
                </p>
                <ul class="pending-list">
                    <For
                        each=move || items.get()
                        key=|(request_id, _)| request_id.clone()
                        children=move |(request_id, summary)| {
                            let label = format!("[{request_id}] {summary}");
                            view! { <li>{label}</li> }
                        }
                    />
                </ul>
            </section>
        </Show>
    }
}
