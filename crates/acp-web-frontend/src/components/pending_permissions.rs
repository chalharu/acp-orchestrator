//! Pending permission panel rendered below the transcript.

use acp_contracts_permissions::PermissionRequest;
use leptos::prelude::*;

#[component]
pub(crate) fn ChatActivity(
    #[prop(into)] items: Signal<Vec<PermissionRequest>>,
    #[prop(into)] busy: Signal<bool>,
    on_approve: Callback<String>,
    on_deny: Callback<String>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <Show when=move || !items.get().is_empty() fallback=move || view! { <></> }>
            <section class="chat-activity" aria-live="polite">
                <p class="chat-activity__section-label">"Pending permissions"</p>
                <ul class="pending-list chat-activity__pending-list">
                    <For
                        each=move || items.get()
                        key=|item| item.request_id.clone()
                        children=move |item| {
                            view! {
                                <PendingPermissionItem
                                    request_id=item.request_id
                                    summary=item.summary
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
    view! {
        <li class="pending-list__item">
            <p class="pending-list__label">"Permission required"</p>
            <p class="pending-list__summary">{summary}</p>
            <div class="pending-list__actions">
                <PendingPermissionActionButton
                    request_id=request_id.clone()
                    label="Approve"
                    button_class="pending-list__button--primary"
                    busy=busy
                    on_click=on_approve
                />
                <PendingPermissionActionButton
                    request_id=request_id
                    label="Deny"
                    button_class="pending-list__button--secondary"
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
    button_class: &'static str,
    #[prop(into)] busy: Signal<bool>,
    on_click: Callback<String>,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class=button_class
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
                "Cancel"
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    fn permission(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.to_string(),
            summary: format!("summary for {id}"),
        }
    }

    #[test]
    fn chat_activity_builds_for_pending_permissions() {
        let owner = Owner::new();
        owner.with(|| {
            let pending = permission("req-1");
            let items = Signal::derive(move || vec![pending.clone()]);
            let busy = Signal::derive(|| false);

            let _ = view! {
                <ChatActivity
                    items=items
                    busy=busy
                    on_approve=Callback::new(|_: String| {})
                    on_deny=Callback::new(|_: String| {})
                    on_cancel=Callback::new(|()| {})
                />
            };
        });
    }

    #[test]
    fn pending_permission_item_builds_buttons_for_approve_and_deny() {
        let owner = Owner::new();
        owner.with(|| {
            let busy = Signal::derive(|| true);

            let _ = view! {
                <PendingPermissionItem
                    request_id="req-2".to_string()
                    summary="Need approval".to_string()
                    busy=busy
                    on_approve=Callback::new(|_: String| {})
                    on_deny=Callback::new(|_: String| {})
                />
            };
        });
    }

    #[test]
    fn pending_permission_footer_builds_cancel_action() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <PendingPermissionFooter
                    busy=Signal::derive(|| false)
                    on_cancel=Callback::new(|()| {})
                />
            };
        });
    }

    // -----------------------------------------------------------------------
    // ChatActivity with empty items (covers the Show fallback path)
    // -----------------------------------------------------------------------

    #[test]
    fn chat_activity_renders_nothing_when_items_are_empty() {
        let owner = Owner::new();
        owner.with(|| {
            let items = Signal::derive(Vec::<PermissionRequest>::new);
            let busy = Signal::derive(|| false);

            let _ = view! {
                <ChatActivity
                    items=items
                    busy=busy
                    on_approve=Callback::new(|_: String| {})
                    on_deny=Callback::new(|_: String| {})
                    on_cancel=Callback::new(|()| {})
                />
            };
        });
    }

    // -----------------------------------------------------------------------
    // PendingPermissionActionButton (direct build)
    // -----------------------------------------------------------------------

    #[test]
    fn pending_permission_action_button_builds_for_approve_and_deny() {
        let owner = Owner::new();
        owner.with(|| {
            let busy_false = Signal::derive(|| false);
            let busy_true = Signal::derive(|| true);

            let _ = view! {
                <PendingPermissionActionButton
                    request_id="req-3".to_string()
                    label="Approve"
                    button_class="btn--primary"
                    busy=busy_false
                    on_click=Callback::new(|_: String| {})
                />
            };
            // Also exercise the disabled (busy=true) variant.
            let _ = view! {
                <PendingPermissionActionButton
                    request_id="req-4".to_string()
                    label="Deny"
                    button_class="btn--secondary"
                    busy=busy_true
                    on_click=Callback::new(|_: String| {})
                />
            };
        });
    }
}
