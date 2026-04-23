use leptos::prelude::*;

#[cfg(target_family = "wasm")]
#[component]
pub(super) fn SessionSidebarStatus(
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] has_session_items: Signal<bool>,
) -> impl IntoView {
    view! {
        <Show when=move || session_list_error.get().is_some() && has_session_items.get()>
            <p class="session-sidebar__status muted">
                {move || session_list_error.get().unwrap_or_default()}
            </p>
        </Show>
    }
}

#[cfg(not(target_family = "wasm"))]
#[component]
pub(super) fn SessionSidebarStatus(
    #[prop(into)] session_list_error: Signal<Option<String>>,
    #[prop(into)] has_session_items: Signal<bool>,
) -> impl IntoView {
    let error = session_list_error.get_untracked();
    let has_items = has_session_items.get_untracked();

    if let (true, Some(message)) = (has_items, error) {
        view! { <p class="session-sidebar__status muted">{message}</p> }.into_any()
    } else {
        view! { <span hidden=true></span> }.into_any()
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::SessionSidebarStatus;

    #[test]
    fn sidebar_status_component_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <SessionSidebarStatus
                    session_list_error=Signal::derive(|| Some("temporary".to_string()))
                    has_session_items=Signal::derive(|| true)
                />
            };
        });
    }
}
