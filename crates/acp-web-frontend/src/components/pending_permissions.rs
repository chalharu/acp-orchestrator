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
            <section class="panel pending-panel" aria-live="polite">
                <h2>"Pending permissions"</h2>
                <p class="muted">
                    "Resolve the pending request to continue this turn, or cancel it."
                </p>
                <ul class="pending-list">
                    <For
                        each=move || items.get()
                        key=|(request_id, _)| request_id.clone()
                        children=move |(request_id, summary)| {
                            view! {
                                <PendingPermissionItem
                                    request_id=request_id
                                    summary=summary
                                    busy=busy
                                    on_approve=on_approve
                                    on_deny=on_deny
                                />
                            }
                        }
                    />
                </ul>
                <PendingPermissionFooter busy=busy on_cancel=on_cancel />
            </section>
        </Show>
    }
}

#[component]
fn PendingPermissionItem(
    request_id: String,
    summary: String,
    #[prop(into)] busy: Signal<bool>,
    on_approve: Callback<String>,
    on_deny: Callback<String>,
) -> impl IntoView {
    let label = format!("[{request_id}] {summary}");

    view! {
        <li class="pending-list__item">
            <p class="pending-list__summary">{label}</p>
            <div class="pending-list__actions">
                <PendingPermissionActionButton
                    request_id=request_id.clone()
                    label="Approve"
                    button_class=None
                    busy=busy
                    on_click=on_approve
                />
                <PendingPermissionActionButton
                    request_id=request_id
                    label="Deny"
                    button_class=Some("pending-list__button--secondary")
                    busy=busy
                    on_click=on_deny
                />
            </div>
        </li>
    }
}

#[component]
fn PendingPermissionActionButton(
    request_id: String,
    label: &'static str,
    button_class: Option<&'static str>,
    #[prop(into)] busy: Signal<bool>,
    on_click: Callback<String>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class=button_class.unwrap_or_default()
            on:click=move |_| on_click.run(request_id.clone())
            prop:disabled=move || busy.get()
        >
            {label}
        </button>
    }
}

#[component]
fn PendingPermissionFooter(
    #[prop(into)] busy: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
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
    }
}
