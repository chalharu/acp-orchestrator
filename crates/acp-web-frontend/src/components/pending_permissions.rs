//! Pending permission panel rendered below the transcript.

use leptos::prelude::*;

use crate::PendingPermission;

#[component]
pub fn ChatActivity(
    #[prop(into)] items: Signal<Vec<PendingPermission>>,
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
