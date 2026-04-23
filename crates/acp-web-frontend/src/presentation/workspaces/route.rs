use leptos::prelude::*;

use crate::{application::auth::WorkspacesRouteAccess, components::ErrorBanner};

use super::{
    create_workspace::CreateWorkspaceSection,
    registry::WorkspaceRegistrySection,
    shared::{WorkspacesPageState, initialize_workspaces_page},
};

#[component]
pub fn WorkspacesPage() -> impl IntoView {
    let state = WorkspacesPageState::new();
    initialize_workspaces_page(state);

    workspaces_page_shell(state)
}

#[cfg(target_family = "wasm")]
fn workspaces_page_shell(state: WorkspacesPageState) -> impl IntoView {
    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Workspaces"</h1>
                    <div class="account-panel__header-actions">
                        <a href="/app/">"Back to chat"</a>
                    </div>
                </div>
                <Show when=move || state.notice.get().is_some()>
                    <p class="account-notice" role="status">
                        {move || state.notice.get().unwrap_or_default()}
                    </p>
                </Show>
                <WorkspacesPageContent state />
            </section>
        </main>
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspaces_notice_view(notice: Option<String>) -> AnyView {
    if let Some(notice) = notice {
        view! {
            <p class="account-notice" role="status">
                {notice}
            </p>
        }
        .into_any()
    } else {
        ().into_any()
    }
}

#[cfg(not(target_family = "wasm"))]
fn workspaces_page_shell(state: WorkspacesPageState) -> impl IntoView {
    let notice_view = workspaces_notice_view(state.notice.get_untracked());

    view! {
        <main class="app-shell account-shell">
            <ErrorBanner message=state.error />
            <section class="panel account-panel">
                <div class="account-panel__header">
                    <h1>"Workspaces"</h1>
                    <div class="account-panel__header-actions">
                        <a href="/app/">"Back to chat"</a>
                    </div>
                </div>
                {notice_view}
                <WorkspacesPageContent state />
            </section>
        </main>
    }
}

#[component]
fn WorkspacesPageContent(state: WorkspacesPageState) -> impl IntoView {
    workspaces_page_content(state)
}

#[cfg(target_family = "wasm")]
fn workspaces_page_content(state: WorkspacesPageState) -> impl IntoView {
    move || workspaces_page_content_body(state.access.get(), state)
}

#[cfg(not(target_family = "wasm"))]
fn workspaces_page_content(state: WorkspacesPageState) -> impl IntoView {
    workspaces_page_content_body(state.access.get_untracked(), state)
}

fn workspaces_page_content_body(
    access: Option<WorkspacesRouteAccess>,
    state: WorkspacesPageState,
) -> AnyView {
    match access {
        Some(WorkspacesRouteAccess::SignedIn) => view! {
            <CreateWorkspaceSection state />
            <WorkspaceRegistrySection state />
        }
        .into_any(),
        Some(WorkspacesRouteAccess::RegisterRequired) => view! {
            <p class="muted">
                "Bootstrap registration is still required. "
                <a href="/app/register/">"Create the first account."</a>
            </p>
        }
        .into_any(),
        Some(WorkspacesRouteAccess::SignInRequired) => view! {
            <p class="muted">
                "Sign in is required before managing workspaces. "
                <a href="/app/sign-in/">"Open sign-in."</a>
            </p>
        }
        .into_any(),
        None => view! { <p class="muted">"Checking access…"</p> }.into_any(),
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn workspaces_page_content_builds_for_each_access_state() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();

            state.access.set(Some(WorkspacesRouteAccess::SignedIn));
            let _ = view! { <WorkspacesPageContent state=state /> };

            state
                .access
                .set(Some(WorkspacesRouteAccess::RegisterRequired));
            let _ = view! { <WorkspacesPageContent state=state /> };

            state
                .access
                .set(Some(WorkspacesRouteAccess::SignInRequired));
            let _ = view! { <WorkspacesPageContent state=state /> };
        });
    }

    #[test]
    fn workspaces_page_and_shell_render_host_safe_views() {
        let owner = Owner::new();
        owner.with(|| {
            let state = WorkspacesPageState::new();
            state.notice.set(Some("Workspace updated.".to_string()));
            state.access.set(Some(WorkspacesRouteAccess::SignedIn));
            let _ = workspaces_page_shell(state);
            let _ = view! { <WorkspacesPage /> };
        });
    }
}
