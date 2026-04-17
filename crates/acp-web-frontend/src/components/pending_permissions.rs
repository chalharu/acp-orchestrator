//! Pending permission panel.

use leptos::prelude::*;

/// Displays pending tool-permission requests.
///
/// Each item is `(request_id, summary)` sourced from
/// `acp_contracts::PermissionRequest`.
#[component]
pub fn PendingPermissions(
    #[prop(into)] items: Signal<Vec<(String, String)>>,
    #[prop(into)] busy: Signal<bool>,
    on_approve: Callback<String>,
    on_deny: Callback<String>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <Show when=move || !items.get().is_empty()>
            <section
                class="panel pending-panel"
                aria-live="polite"
            >
                <h2>"Pending permissions"</h2>
                <p class="muted">
                    "Resolve the pending request to continue this turn, or cancel it."
                </p>
                <ul class="pending-list">
                    <For
                        each=move || items.get()
                        key=|(request_id, _)| request_id.clone()
                        children=move |(request_id, summary)| {
                            let label = format!("[{request_id}] {summary}");
                            let approve_request_id = request_id.clone();
                            let deny_request_id = request_id.clone();
                            view! {
                                <li class="pending-list__item">
                                    <p class="pending-list__summary">{label}</p>
                                    <div class="pending-list__actions">
                                        <button
                                            type="button"
                                            on:click=move |_| on_approve.run(approve_request_id.clone())
                                            prop:disabled=move || busy.get()
                                        >
                                            "Approve"
                                        </button>
                                        <button
                                            type="button"
                                            class="pending-list__button--secondary"
                                            on:click=move |_| on_deny.run(deny_request_id.clone())
                                            prop:disabled=move || busy.get()
                                        >
                                            "Deny"
                                        </button>
                                    </div>
                                </li>
                            }
                        }
                    />
                </ul>
                <div class="pending-panel__footer">
                    <button
                        type="button"
                        class="pending-list__button--secondary"
                        on:click=move |_| on_cancel.run(())
                        prop:disabled=move || busy.get()
                    >
                        "Cancel turn"
                    </button>
                </div>
            </section>
        </Show>
    }
}
