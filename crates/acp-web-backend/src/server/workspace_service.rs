use crate::{
    contract_workspaces::{WorkspaceDetail, WorkspaceSummary},
    workspace_records::WorkspaceRecord,
};

pub(super) fn workspace_summary(record: WorkspaceRecord) -> WorkspaceSummary {
    WorkspaceSummary {
        workspace_id: record.workspace_id,
        name: record.name,
        upstream_url: record.upstream_url,
        default_ref: record.default_ref,
        bootstrap_kind: record.bootstrap_kind,
        status: record.status,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

pub(super) fn workspace_detail(record: WorkspaceRecord) -> WorkspaceDetail {
    WorkspaceDetail {
        workspace_id: record.workspace_id,
        name: record.name,
        upstream_url: record.upstream_url,
        default_ref: record.default_ref,
        credential_reference_id: record.credential_reference_id,
        bootstrap_kind: record.bootstrap_kind,
        status: record.status,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}
