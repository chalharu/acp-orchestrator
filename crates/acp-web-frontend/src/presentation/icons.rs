use leptos::prelude::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppIcon {
    Accounts,
    Workspaces,
}

pub(crate) fn app_icon_view(icon: AppIcon) -> AnyView {
    match icon {
        AppIcon::Accounts => accounts_icon_view(),
        AppIcon::Workspaces => workspaces_icon_view(),
    }
}

fn accounts_icon_view() -> AnyView {
    view! {
        <svg
            class="session-sidebar__icon-svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <circle cx="12" cy="12" r="8.25" />
            <circle cx="12" cy="9" r="2.75" />
            <path d="M7.75 16.75c.91-1.57 2.55-2.5 4.25-2.5s3.34.93 4.25 2.5" />
        </svg>
    }
    .into_any()
}

fn workspaces_icon_view() -> AnyView {
    view! {
        <svg
            class="session-sidebar__icon-svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="M4.25 7A2.75 2.75 0 0 1 7 4.25h3.09c.49 0 .95.19 1.29.53l1.34 1.34c.34.34.8.54 1.29.54H17A2.75 2.75 0 0 1 19.75 9.45v6.8A2.75 2.75 0 0 1 17 19H7a2.75 2.75 0 0 1-2.75-2.75V7Z" />
            <path d="M4.25 9.25h15.5" />
        </svg>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn app_icons_render_host_safe_views() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = app_icon_view(AppIcon::Workspaces);
            let _ = app_icon_view(AppIcon::Accounts);
        });
    }
}
