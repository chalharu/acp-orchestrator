use leptos::prelude::*;

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarStatus(
    #[prop(into)] workspace_message: Signal<Option<String>>,
    #[prop(into)] sidebar_error: Signal<Option<String>>,
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] has_session_items: Signal<bool>,
) -> impl IntoView {
    view! {
        <Show when=move || workspace_message.get().is_some()>
            <p class="session-sidebar__workspace muted" aria-label="Current workspace">
                {move || format!("Workspace: {}", workspace_message.get().unwrap_or_default())}
            </p>
        </Show>
        <Show when=move || {
            sidebar_status_message(
                sidebar_error.get(),
                session_list_error.get(),
                has_session_items.get(),
            )
            .is_some()
        }>
            <p class="session-sidebar__status muted">
                {move || {
                    sidebar_status_message(
                        sidebar_error.get(),
                        session_list_error.get(),
                        has_session_items.get(),
                    )
                    .unwrap_or_default()
                }}
            </p>
        </Show>
    }
}

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarStatus(
    #[prop(into)] workspace_message: Signal<Option<String>>,
    #[prop(into)] sidebar_error: Signal<Option<String>>,
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] has_session_items: Signal<bool>,
) -> impl IntoView {
    let workspace = workspace_message.get_untracked();
    let sidebar_message = sidebar_error.get_untracked();
    let error = session_list_error.get_untracked();
    let has_items = has_session_items.get_untracked();

    view! {
        <>
            {workspace
                .map(|workspace| {
                    view! {
                        <p class="session-sidebar__workspace muted" aria-label="Current workspace">
                            {format!("Workspace: {workspace}")}
                        </p>
                    }
                    .into_any()
                })
                .unwrap_or_else(|| view! { <span hidden=true></span> }.into_any())}
            {if let Some(message) = sidebar_status_message(sidebar_message, error, has_items) {
                view! { <p class="session-sidebar__status muted">{message}</p> }.into_any()
            } else {
                view! { <span hidden=true></span> }.into_any()
            }}
        </>
    }
}

fn sidebar_status_message(
    sidebar_error: Option<String>,
    session_list_error: Option<String>,
    has_session_items: bool,
) -> Option<String> {
    sidebar_error.or_else(|| {
        if has_session_items {
            session_list_error
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::{SessionSidebarStatus, sidebar_status_message};

    #[test]
    fn sidebar_status_component_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <SessionSidebarStatus
                    workspace_message=Signal::derive(|| Some("Workspace A".to_string()))
                    sidebar_error=Signal::derive(|| None::<String>)
                    session_list_error=Signal::derive(|| Some("temporary".to_string()))
                    has_session_items=Signal::derive(|| true)
                />
            };
            let _ = view! {
                <SessionSidebarStatus
                    workspace_message=Signal::derive(|| None::<String>)
                    sidebar_error=Signal::derive(|| Some("sidebar".to_string()))
                    session_list_error=Signal::derive(|| None::<String>)
                    has_session_items=Signal::derive(|| false)
                />
            };
        });
    }

    #[test]
    fn sidebar_status_message_prefers_sidebar_errors() {
        assert_eq!(
            sidebar_status_message(
                Some("sidebar".to_string()),
                Some("list".to_string()),
                true,
            ),
            Some("sidebar".to_string())
        );
        assert_eq!(
            sidebar_status_message(None, Some("list".to_string()), true),
            Some("list".to_string())
        );
        assert_eq!(sidebar_status_message(None, Some("list".to_string()), false), None);
    }
}
