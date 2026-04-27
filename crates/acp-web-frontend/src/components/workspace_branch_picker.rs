use acp_contracts_workspaces::WorkspaceBranch;
use leptos::prelude::*;

pub(crate) fn workspace_branch_status_message(loading_branches: bool) -> &'static str {
    if loading_branches {
        "Loading branches for this workspace..."
    } else {
        "Choose a branch for this chat."
    }
}

pub(crate) fn workspace_branch_select_field(
    branches: Signal<Vec<WorkspaceBranch>>,
    selected_branch: Signal<String>,
    loading_branches: Signal<bool>,
    on_change: impl Fn(web_sys::Event) + Copy + 'static,
) -> impl IntoView {
    view! {
        <label class="account-form__field">
            <span>"Branch"</span>
            <select
                class="workspace-branch-select"
                prop:value=selected_branch
                on:change=on_change
                prop:disabled=move || loading_branches.get() || branches.get().is_empty()
            >
                <option value="">
                    {move || {
                        if loading_branches.get() {
                            "Loading branches..."
                        } else {
                            "Choose a branch"
                        }
                    }}
                </option>
                {move || {
                    branches
                        .get()
                        .into_iter()
                        .map(|branch| {
                            let label = branch.name;
                            let value = branch.ref_name;
                            view! { <option value=value>{label}</option> }
                        })
                        .collect_view()
                }}
            </select>
            <Show when=move || !loading_branches.get() && branches.get().is_empty()>
                <span class="workspace-field__hint">
                    "No branches are available for this workspace."
                </span>
            </Show>
        </label>
    }
}

pub(crate) fn workspace_branch_modal_actions(
    submit_label: impl Fn() -> &'static str + Copy + Send + Sync + 'static,
    busy: Signal<bool>,
    loading_branches: Signal<bool>,
    selected_branch: Signal<String>,
    branches: Signal<Vec<WorkspaceBranch>>,
    on_cancel: impl Fn(web_sys::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    let submit_disabled = Signal::derive(move || {
        busy.get()
            || loading_branches.get()
            || selected_branch.get().trim().is_empty()
            || branches.get().is_empty()
    });

    view! {
        <div class="workspace-modal__actions">
            <button
                type="submit"
                class="account-form__submit"
                prop:disabled=move || submit_disabled.get()
            >
                {move || submit_label()}
            </button>
            <button
                type="button"
                class="account-form__cancel"
                on:click=on_cancel
                prop:disabled=move || busy.get()
            >
                "Cancel"
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    fn sample_branch() -> WorkspaceBranch {
        WorkspaceBranch {
            name: "main".to_string(),
            ref_name: "refs/heads/main".to_string(),
        }
    }

    #[test]
    fn workspace_branch_status_message_reflects_loading_state() {
        assert_eq!(
            workspace_branch_status_message(true),
            "Loading branches for this workspace..."
        );
        assert_eq!(
            workspace_branch_status_message(false),
            "Choose a branch for this chat."
        );
    }

    #[test]
    fn workspace_branch_select_field_renders_loading_populated_and_empty_states() {
        let owner = Owner::new();
        owner.with(|| {
            let loading_html = workspace_branch_select_field(
                Signal::derive(Vec::<WorkspaceBranch>::new),
                Signal::derive(String::new),
                Signal::derive(|| true),
                |_| {},
            )
            .to_html();
            assert!(loading_html.contains("Loading branches..."));

            let populated_html = workspace_branch_select_field(
                Signal::derive(|| vec![sample_branch()]),
                Signal::derive(|| "refs/heads/main".to_string()),
                Signal::derive(|| false),
                |_| {},
            )
            .to_html();
            assert!(populated_html.contains("Choose a branch"));
            assert!(populated_html.contains("refs/heads/main"));
            assert!(populated_html.contains(">main<"));

            let empty_html = workspace_branch_select_field(
                Signal::derive(Vec::<WorkspaceBranch>::new),
                Signal::derive(String::new),
                Signal::derive(|| false),
                |_| {},
            )
            .to_html();
            assert!(empty_html.contains("No branches are available for this workspace."));
        });
    }

    #[test]
    fn workspace_branch_modal_actions_render_ready_and_disabled_states() {
        let owner = Owner::new();
        owner.with(|| {
            let ready_html = workspace_branch_modal_actions(
                || "New chat",
                Signal::derive(|| false),
                Signal::derive(|| false),
                Signal::derive(|| "refs/heads/main".to_string()),
                Signal::derive(|| vec![sample_branch()]),
                |_| {},
            )
            .to_html();
            assert!(ready_html.contains("New chat"));
            assert!(ready_html.contains("Cancel"));

            let disabled_html = workspace_branch_modal_actions(
                || "Creating...",
                Signal::derive(|| false),
                Signal::derive(|| false),
                Signal::derive(String::new),
                Signal::derive(Vec::<WorkspaceBranch>::new),
                |_| {},
            )
            .to_html();
            assert!(disabled_html.contains("Creating..."));
            assert!(disabled_html.contains("disabled"));
        });
    }
}
