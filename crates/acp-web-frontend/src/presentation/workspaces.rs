mod create_workspace;
mod registry;
mod route;
mod shared;

pub use route::WorkspacesPage;
pub(in crate::presentation) use shared::workspaces_path_with_return_to;
