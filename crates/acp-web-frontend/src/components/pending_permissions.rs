//! Tool activity panel.

use acp_contracts::CompletionCandidate;
use leptos::prelude::*;

use crate::{PendingPermission, ToolActivityEntry};

#[component]
pub fn ToolActivityPanel(
    #[prop(into)] items: Signal<Vec<PendingPermission>>,
    #[prop(into)] activity: Signal<Vec<ToolActivityEntry>>,
    #[prop(into)] busy: Signal<bool>,
    #[prop(into)] slash_help_hint: Signal<Option<String>>,
    on_approve: Callback<String>,
    on_deny: Callback<String>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <section class="panel tool-activity-panel" aria-live="polite">
            <div class="tool-activity-panel__header">
                <div>
                    <p class="tool-activity-panel__eyebrow">"Tool activity"</p>
                    <h2 class="tool-activity-panel__title">"Status, permissions, and slash help"</h2>
                </div>
                <Show when=move || slash_help_hint.get().is_some()>
                    <p class="tool-activity-panel__hint">
                        {move || slash_help_hint.get().unwrap_or_default()}
                    </p>
                </Show>
            </div>

            <Show when=move || !items.get().is_empty() fallback=move || view! { <></> }>
                <section class="tool-activity-panel__section">
                    <p class="tool-activity-panel__section-label">"Pending permissions"</p>
                    <ul class="pending-list">
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

            <section class="tool-activity-panel__section">
                <p class="tool-activity-panel__section-label">"Recent activity"</p>
                <Show
                    when=move || !activity.get().is_empty()
                    fallback=move || {
                        view! {
                            <p class="tool-activity-panel__empty muted">
                                "No tool activity yet. Type "
                                <code>/</code>
                                " to browse commands."
                            </p>
                        }
                    }
                >
                    <ul class="tool-activity-list">
                        <For
                            each=move || activity.get()
                            key=|item| item.id.clone()
                            children=move |item| view! { <ToolActivityItem item=item /> }
                        />
                    </ul>
                </Show>
            </section>
        </section>
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

#[component]
fn ToolActivityItem(item: ToolActivityEntry) -> impl IntoView {
    let item_class = format!("tool-activity-list__item {}", item.kind.css_class());
    let ToolActivityEntry {
        title,
        detail,
        commands,
        ..
    } = item;
    let has_detail = !detail.is_empty();
    let has_commands = !commands.is_empty();
    let commands_for_rows = commands;

    view! {
        <li class=item_class>
            <div class="tool-activity-list__body">
                <p class="tool-activity-list__title">{title}</p>
                {move || {
                    if has_detail {
                        view! { <p class="tool-activity-list__detail">{detail.clone()}</p> }.into_any()
                    } else {
                        ().into_any()
                    }
                }}
                {move || {
                    if has_commands {
                        let command_rows = commands_for_rows
                            .clone()
                            .into_iter()
                            .map(|command| view! { <ToolActivityCommandRow command=command /> })
                            .collect_view();
                        view! { <ul class="tool-activity-list__commands">{command_rows}</ul> }.into_any()
                    } else {
                        ().into_any()
                    }
                }}
            </div>
        </li>
    }
}

#[component]
fn ToolActivityCommandRow(command: CompletionCandidate) -> impl IntoView {
    let CompletionCandidate { label, detail, .. } = command;

    view! {
        <li class="tool-activity-list__command">
            <code>{label}</code>
            <span>{detail}</span>
        </li>
    }
}
