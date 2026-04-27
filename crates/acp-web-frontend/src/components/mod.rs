//! Reusable Leptos UI components for the ACP web frontend.

pub(crate) mod composer;
mod composer_footer;
mod composer_palette;
pub(crate) mod error_banner;
pub(crate) mod pending_permissions;
pub(crate) mod transcript;
#[cfg(target_family = "wasm")]
mod workspace_branch_picker;

pub(crate) use error_banner::ErrorBanner;
#[cfg(target_family = "wasm")]
pub(crate) use workspace_branch_picker::{
    workspace_branch_modal_actions, workspace_branch_select_field, workspace_branch_status_message,
};
