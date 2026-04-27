use leptos::prelude::*;
use leptos_icons::Icon;

use icondata::{
    FaArrowLeftLongSolid, FaArrowRightFromBracketSolid, FaFloppyDiskSolid, FaFolderPlusSolid,
    FaFolderTreeSolid, FaPaperPlaneSolid, FaPenToSquareSolid, FaPlusSolid, FaSpinnerSolid,
    FaTrashCanSolid, FaUserGearSolid, FaXmarkSolid, Icon as IconData,
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
    Send,
    SignOut,
    Workspaces,
}

pub(crate) fn app_icon_view(icon: AppIcon) -> AnyView {
    app_icon_shell(
        app_icon_class(icon),
        view! { <Icon icon=app_icon_data(icon) width="1em" height="1em" /> },
    )
}

fn app_icon_class(icon: AppIcon) -> &'static str {
    if matches!(icon, AppIcon::Busy) {
        "app-icon app-icon--spin"
    } else {
        "app-icon"
    }
}

fn app_icon_data(icon: AppIcon) -> IconData {
    match icon {
        AppIcon::Accounts => FaUserGearSolid,
        AppIcon::BackToChat => FaArrowLeftLongSolid,
        AppIcon::Busy => FaSpinnerSolid,
        AppIcon::Cancel => FaXmarkSolid,
        AppIcon::CreateWorkspace => FaFolderPlusSolid,
        AppIcon::Delete => FaTrashCanSolid,
        AppIcon::NewChat => FaPlusSolid,
        AppIcon::Rename => FaPenToSquareSolid,
        AppIcon::Save => FaFloppyDiskSolid,
        AppIcon::Send => FaPaperPlaneSolid,
        AppIcon::SignOut => FaArrowRightFromBracketSolid,
        AppIcon::Workspaces => FaFolderTreeSolid,
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
            let _ = app_icon_view(AppIcon::Send);
            let _ = app_icon_view(AppIcon::SignOut);
            let _ = app_icon_view(AppIcon::Workspaces);
            let _ = app_icon_view(AppIcon::Accounts);
        });
    }
}
