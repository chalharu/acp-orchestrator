use leptos::prelude::*;
use leptos_icons::Icon;

use icondata::{
    FaArrowLeftLongSolid, FaArrowRightFromBracketSolid, FaFloppyDiskSolid, FaFolderPlusSolid,
    FaFolderTreeSolid, FaPenToSquareSolid, FaPlusSolid, FaSpinnerSolid, FaTrashCanSolid,
    FaUserGearSolid, FaXmarkSolid,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppIcon {
    Accounts,
    BackToChat,
    Busy,
    Cancel,
    CreateWorkspace,
    Delete,
    NewChat,
    Rename,
    Save,
    SignOut,
    Workspaces,
}

pub(crate) fn app_icon_view(icon: AppIcon) -> AnyView {
    let class = if matches!(icon, AppIcon::Busy) {
        "app-icon app-icon--spin"
    } else {
        "app-icon"
    };

    match icon {
        AppIcon::Accounts => app_icon_shell(
            class,
            view! { <Icon icon=FaUserGearSolid width="1em" height="1em" /> },
        ),
        AppIcon::BackToChat => app_icon_shell(
            class,
            view! { <Icon icon=FaArrowLeftLongSolid width="1em" height="1em" /> },
        ),
        AppIcon::Busy => app_icon_shell(
            class,
            view! { <Icon icon=FaSpinnerSolid width="1em" height="1em" /> },
        ),
        AppIcon::Cancel => app_icon_shell(
            class,
            view! { <Icon icon=FaXmarkSolid width="1em" height="1em" /> },
        ),
        AppIcon::CreateWorkspace => app_icon_shell(
            class,
            view! { <Icon icon=FaFolderPlusSolid width="1em" height="1em" /> },
        ),
        AppIcon::Delete => app_icon_shell(
            class,
            view! { <Icon icon=FaTrashCanSolid width="1em" height="1em" /> },
        ),
        AppIcon::NewChat => app_icon_shell(
            class,
            view! { <Icon icon=FaPlusSolid width="1em" height="1em" /> },
        ),
        AppIcon::Rename => app_icon_shell(
            class,
            view! { <Icon icon=FaPenToSquareSolid width="1em" height="1em" /> },
        ),
        AppIcon::Save => app_icon_shell(
            class,
            view! { <Icon icon=FaFloppyDiskSolid width="1em" height="1em" /> },
        ),
        AppIcon::SignOut => app_icon_shell(
            class,
            view! { <Icon icon=FaArrowRightFromBracketSolid width="1em" height="1em" /> },
        ),
        AppIcon::Workspaces => app_icon_shell(
            class,
            view! { <Icon icon=FaFolderTreeSolid width="1em" height="1em" /> },
        ),
    }
}

fn app_icon_shell(class: &'static str, icon: impl IntoView + 'static) -> AnyView {
    view! {
        <span class=class aria-hidden="true">
            {icon}
        </span>
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
            let _ = app_icon_view(AppIcon::BackToChat);
            let _ = app_icon_view(AppIcon::Busy);
            let _ = app_icon_view(AppIcon::Cancel);
            let _ = app_icon_view(AppIcon::CreateWorkspace);
            let _ = app_icon_view(AppIcon::Delete);
            let _ = app_icon_view(AppIcon::NewChat);
            let _ = app_icon_view(AppIcon::Rename);
            let _ = app_icon_view(AppIcon::Save);
            let _ = app_icon_view(AppIcon::SignOut);
            let _ = app_icon_view(AppIcon::Workspaces);
            let _ = app_icon_view(AppIcon::Accounts);
        });
    }
}
