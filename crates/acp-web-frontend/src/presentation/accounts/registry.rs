#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use acp_contracts_accounts::LocalAccount;
use leptos::prelude::*;

use crate::application::auth::{
    AccountCapabilities, AccountConstraintReason, account_capabilities,
};
#[cfg(target_family = "wasm")]
use crate::infrastructure::api;

use super::shared::{AccountsPageState, event_target_checked, spawn_account_reload};

#[component]
#[cfg(target_family = "wasm")]
pub(super) fn CurrentAccountsSection(state: AccountsPageState) -> impl IntoView {
    let account_count = Signal::derive(move || state.accounts.get().len());

    view! {
        <div class="account-panel__section account-panel__section--registry">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Account registry"</h2>
                    <p class="muted">
                        "User names stay fixed. Adjust passwords and admin access row by row."
                    </p>
                </div>
                <p class="account-panel__summary">
                    {move || account_count_label(account_count.get())}
                </p>
            </div>
            <Show
                when=move || !state.loading_accounts.get()
                fallback=|| view! { <p class="muted">"Loading accounts…"</p> }
            >
                <div class="account-table-wrap">
                    <table class="account-table">
                        <caption class="sr-only">"Local accounts and admin controls"</caption>
                        <thead>
                            <tr>
                                <th scope="col">"Account"</th>
                                <th scope="col">"State"</th>
                                <th scope="col">"Created"</th>
                                <th scope="col">"Password reset"</th>
                                <th scope="col">"Admin access"</th>
                                <th scope="col">"Actions"</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For
                                each=move || state.accounts.get()
                                key=|account| account.user_id.clone()
                                children=move |account| view! { <AccountRow account state /> }
                            />
                        </tbody>
                    </table>
                </div>
            </Show>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
pub(super) fn CurrentAccountsSection(state: AccountsPageState) -> impl IntoView {
    let loading_accounts = state.loading_accounts.get_untracked();
    let summary = account_count_label(state.accounts.get_untracked().len());
    let content = if loading_accounts {
        view! { <p class="muted">"Loading accounts…"</p> }.into_any()
    } else {
        let rows = state
            .accounts
            .get_untracked()
            .into_iter()
            .map(|account| view! { <AccountRow account state /> })
            .collect_view()
            .into_any();
        view! {
            <div class="account-table-wrap">
                <table class="account-table">
                    <caption class="sr-only">"Local accounts and admin controls"</caption>
                    <thead>
                        <tr>
                            <th scope="col">"Account"</th>
                            <th scope="col">"State"</th>
                            <th scope="col">"Created"</th>
                            <th scope="col">"Password reset"</th>
                            <th scope="col">"Admin access"</th>
                            <th scope="col">"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
        }
        .into_any()
    };

    view! {
        <div class="account-panel__section account-panel__section--registry">
            <div class="account-panel__section-heading">
                <div class="account-panel__section-copy">
                    <h2>"Account registry"</h2>
                    <p class="muted">
                        "User names stay fixed. Adjust passwords and admin access row by row."
                    </p>
                </div>
                <p class="account-panel__summary">{summary}</p>
            </div>
            {content}
        </div>
    }
}

#[component]
fn AccountRow(account: LocalAccount, state: AccountsPageState) -> impl IntoView {
    let password = RwSignal::new(String::new());
    let admin_checked = RwSignal::new(account.is_admin);
    let saving = RwSignal::new(false);
    let deleting = RwSignal::new(false);
    let row_dirty = account_row_dirty_signal(&account, password, admin_checked);
    let capabilities = account_capabilities_signal(&account, state);
    let role_kind = account_role_kind_signal(&account, state);
    let role_label = account_role_label_signal(role_kind);
    let role_badge_class = account_role_badge_class_signal(role_kind);
    let constraint_label = account_constraint_label_signal(capabilities);
    let can_modify = account_can_modify_signal(capabilities);
    let (save_account, delete_account) =
        account_row_action_handlers(&account, state, password, admin_checked, saving, deleting);
    let created_label = account_created_label(&account);
    let username = account.username.clone();

    view! {
        <tr class="account-table__row">
            <td>
                <AccountRowSummary username=username.clone() constraint_label />
            </td>
            <td>
                <AccountStateCell role_label role_badge_class />
            </td>
            <td class="account-table__created">
                <span>{created_label}</span>
            </td>
            <td>
                <AccountPasswordField password username />
            </td>
            <td>
                <AccountAdminToggle admin_checked can_modify />
            </td>
            <td>
                <AccountRowActions
                    saving
                    deleting
                    row_dirty
                    can_modify
                    save_account
                    delete_account
                    constraint_label
                />
            </td>
        </tr>
    }
}

fn account_row_dirty_signal(
    account: &LocalAccount,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
) -> Signal<bool> {
    let row_dirty_account = account.clone();
    Signal::derive(move || {
        !password.get().trim().is_empty() || admin_checked.get() != row_dirty_account.is_admin
    })
}

fn account_capabilities_signal(
    account: &LocalAccount,
    state: AccountsPageState,
) -> Signal<AccountCapabilities> {
    let capabilities_account = account.clone();
    Signal::derive(move || {
        account_capabilities(
            &state.current_user_id.get(),
            &state.accounts.get(),
            &capabilities_account,
        )
    })
}

fn account_role_kind_signal(
    account: &LocalAccount,
    state: AccountsPageState,
) -> Signal<AccountRoleKind> {
    let role_account = account.clone();
    Signal::derive(move || account_role_kind(&role_account, &state.current_user_id.get()))
}

fn account_role_label_signal(role_kind: Signal<AccountRoleKind>) -> Signal<String> {
    Signal::derive(move || account_role_label(role_kind.get()).to_string())
}

fn account_role_badge_class_signal(role_kind: Signal<AccountRoleKind>) -> Signal<String> {
    Signal::derive(move || {
        format!(
            "account-role-pill account-role-pill--{}",
            account_role_badge_modifier(role_kind.get())
        )
    })
}

fn account_constraint_label_signal(capabilities: Signal<AccountCapabilities>) -> Signal<String> {
    Signal::derive(move || account_constraint_label(capabilities.get().constraint).to_string())
}

fn account_can_modify_signal(capabilities: Signal<AccountCapabilities>) -> Signal<bool> {
    Signal::derive(move || capabilities.get().can_modify())
}

fn account_row_action_handlers(
    account: &LocalAccount,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
) -> (Callback<()>, Callback<web_sys::MouseEvent>) {
    let account_id = account.user_id.clone();
    (
        account_save_handler(account_id.clone(), state, password, admin_checked, saving),
        account_delete_handler(account_id, state, deleting),
    )
}

#[component]
fn AccountRowSummary(username: String, constraint_label: Signal<String>) -> impl IntoView {
    view! {
        <div class="account-row__summary">
            <strong class="account-row__name">{username}</strong>
            <Show when=move || !constraint_label.get().is_empty()>
                <span class="account-row__note">{move || constraint_label.get()}</span>
            </Show>
        </div>
    }
}

#[component]
fn AccountStateCell(role_label: Signal<String>, role_badge_class: Signal<String>) -> impl IntoView {
    view! {
        <div class="account-state-cell">
            <span class=move || role_badge_class.get()>{move || role_label.get()}</span>
        </div>
    }
}

#[component]
fn AccountPasswordField(password: RwSignal<String>, username: String) -> impl IntoView {
    let input_label = format!("New password for {username}");
    let sr_label = input_label.clone();

    view! {
        <label class="account-form__field account-form__field--compact">
            <span class="sr-only">{sr_label}</span>
            <input
                type="password"
                placeholder="Leave blank to keep current"
                aria-label=input_label
                prop:value=move || password.get()
                on:input=move |event| password.set(event_target_value(&event))
            />
        </label>
    }
}

#[component]
fn AccountAdminToggle(admin_checked: RwSignal<bool>, can_modify: Signal<bool>) -> impl IntoView {
    view! {
        <label class="account-checkbox account-checkbox--table">
            <input
                type="checkbox"
                prop:checked=move || admin_checked.get()
                prop:disabled=move || !can_modify.get()
                on:change=move |event| admin_checked.set(event_target_checked(&event))
            />
            <span>{move || admin_access_label(admin_checked.get())}</span>
        </label>
    }
}

#[component]
#[cfg(target_family = "wasm")]
fn AccountRowActions(
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
    row_dirty: Signal<bool>,
    can_modify: Signal<bool>,
    save_account: Callback<()>,
    delete_account: Callback<web_sys::MouseEvent>,
    constraint_label: Signal<String>,
) -> impl IntoView {
    let hint_text =
        account_row_hint_signal(saving, deleting, row_dirty, can_modify, constraint_label);

    view! {
        <div class="account-row__actions">
            <button
                type="button"
                prop:disabled=move || saving.get() || !row_dirty.get()
                on:click=move |_| save_account.run(())
            >
                {move || save_button_label(saving.get())}
            </button>
            <button
                type="button"
                class="account-row__delete"
                prop:disabled=move || deleting.get() || !can_modify.get()
                on:click=move |event| delete_account.run(event)
            >
                {move || delete_button_label(deleting.get())}
            </button>
            <p class="account-row__hint">{move || hint_text.get()}</p>
        </div>
    }
}

#[component]
#[cfg(not(target_family = "wasm"))]
fn AccountRowActions(
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
    row_dirty: Signal<bool>,
    can_modify: Signal<bool>,
    save_account: Callback<()>,
    delete_account: Callback<web_sys::MouseEvent>,
    constraint_label: Signal<String>,
) -> impl IntoView {
    let saving_now = saving.get_untracked();
    let deleting_now = deleting.get_untracked();
    let row_dirty_now = row_dirty.get_untracked();
    let can_modify_now = can_modify.get_untracked();
    let hint_text = account_row_hint(
        saving_now,
        deleting_now,
        row_dirty_now,
        can_modify_now,
        constraint_label.get_untracked(),
    );

    view! {
        <div class="account-row__actions">
            <button
                type="button"
                prop:disabled=saving_now || !row_dirty_now
                on:click=move |_| save_account.run(())
            >
                {save_button_label(saving_now)}
            </button>
            <button
                type="button"
                class="account-row__delete"
                prop:disabled=deleting_now || !can_modify_now
                on:click=move |event| delete_account.run(event)
            >
                {delete_button_label(deleting_now)}
            </button>
            <p class="account-row__hint">{hint_text}</p>
        </div>
    }
}

fn account_row_hint_signal(
    saving: RwSignal<bool>,
    deleting: RwSignal<bool>,
    row_dirty: Signal<bool>,
    can_modify: Signal<bool>,
    constraint_label: Signal<String>,
) -> Signal<String> {
    Signal::derive(move || {
        account_row_hint(
            saving.get(),
            deleting.get(),
            row_dirty.get(),
            can_modify.get(),
            constraint_label.get(),
        )
    })
}

fn account_row_hint(
    saving: bool,
    deleting: bool,
    row_dirty: bool,
    can_modify: bool,
    constraint_label: String,
) -> String {
    if saving {
        "Saving changes…".to_string()
    } else if deleting {
        "Removing account…".to_string()
    } else if !row_dirty {
        "No pending changes".to_string()
    } else if !can_modify && !constraint_label.is_empty() {
        constraint_label
    } else {
        "Ready to apply".to_string()
    }
}

#[cfg(target_family = "wasm")]
fn account_save_handler(
    account_user_id: String,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
) -> Callback<()> {
    Callback::new(move |_| {
        if saving.get_untracked() {
            return;
        }

        saving.set(true);
        state.error.set(None);
        state.notice.set(None);
        let password_update = password_update(password.get_untracked());
        let is_admin = Some(admin_checked.get_untracked());
        let account_user_id = account_user_id.clone();
        leptos::task::spawn_local(async move {
            match api::update_account(&account_user_id, password_update, is_admin).await {
                Ok(_) => {
                    password.set(String::new());
                    saving.set(false);
                    state.notice.set(Some("Account updated.".to_string()));
                    spawn_account_reload(state);
                }
                Err(message) => {
                    saving.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(not(target_family = "wasm"))]
fn account_save_handler(
    account_user_id: String,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
) -> Callback<()> {
    Callback::new(move |_| {
        account_save_host(&account_user_id, state, password, admin_checked, saving)
    })
}

#[cfg(target_family = "wasm")]
fn account_delete_handler(
    account_user_id: String,
    state: AccountsPageState,
    deleting: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| {
        if deleting.get_untracked() {
            return;
        }

        deleting.set(true);
        state.error.set(None);
        state.notice.set(None);
        let account_user_id = account_user_id.clone();
        leptos::task::spawn_local(async move {
            match api::delete_account(&account_user_id).await {
                Ok(_) => {
                    deleting.set(false);
                    state.notice.set(Some("Account deleted.".to_string()));
                    spawn_account_reload(state);
                }
                Err(message) => {
                    deleting.set(false);
                    state.error.set(Some(message));
                }
            }
        });
    })
}

#[cfg(not(target_family = "wasm"))]
fn account_delete_handler(
    account_user_id: String,
    state: AccountsPageState,
    deleting: RwSignal<bool>,
) -> Callback<web_sys::MouseEvent> {
    Callback::new(move |_| account_delete_host(&account_user_id, state, deleting))
}

fn password_update(password: String) -> Option<String> {
    if password.trim().is_empty() {
        None
    } else {
        Some(password)
    }
}

fn save_button_label(saving: bool) -> &'static str {
    if saving { "Saving…" } else { "Save" }
}

fn delete_button_label(deleting: bool) -> &'static str {
    if deleting { "Deleting…" } else { "Delete" }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountRoleKind {
    Active,
    Admin,
    Member,
}

fn account_role_kind(account: &LocalAccount, current_user_id: &str) -> AccountRoleKind {
    if account.user_id == current_user_id {
        AccountRoleKind::Active
    } else if account.is_admin {
        AccountRoleKind::Admin
    } else {
        AccountRoleKind::Member
    }
}

fn account_role_label(role_kind: AccountRoleKind) -> &'static str {
    match role_kind {
        AccountRoleKind::Active => "signed in",
        AccountRoleKind::Admin => "admin",
        AccountRoleKind::Member => "member",
    }
}

fn account_role_badge_modifier(role_kind: AccountRoleKind) -> &'static str {
    match role_kind {
        AccountRoleKind::Active => "active",
        AccountRoleKind::Admin => "admin",
        AccountRoleKind::Member => "member",
    }
}

fn account_constraint_label(reason: Option<AccountConstraintReason>) -> &'static str {
    match reason {
        Some(AccountConstraintReason::CurrentUser) => "Signed in on this browser",
        Some(AccountConstraintReason::LastAdmin) => "One admin account must remain",
        None => "",
    }
}

fn account_created_label(account: &LocalAccount) -> String {
    account.created_at.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn account_count_label(count: usize) -> String {
    if count == 1 {
        "1 account".to_string()
    } else {
        format!("{count} accounts")
    }
}

fn admin_access_label(is_admin: bool) -> &'static str {
    if is_admin { "Enabled" } else { "Standard" }
}

#[cfg(not(target_family = "wasm"))]
fn account_save_host(
    account_user_id: &str,
    state: AccountsPageState,
    password: RwSignal<String>,
    admin_checked: RwSignal<bool>,
    saving: RwSignal<bool>,
) {
    if saving.get_untracked() {
        return;
    }

    saving.set(true);
    state.error.set(None);
    state.notice.set(None);
    let _password_update = password_update(password.get_untracked());
    let _is_admin = Some(admin_checked.get_untracked());
    let _account_user_id = account_user_id.to_string();
    password.set(String::new());
    saving.set(false);
    state.notice.set(Some("Account updated.".to_string()));
    spawn_account_reload(state);
}

#[cfg(not(target_family = "wasm"))]
fn account_delete_host(account_user_id: &str, state: AccountsPageState, deleting: RwSignal<bool>) {
    if deleting.get_untracked() {
        return;
    }

    deleting.set(true);
    state.error.set(None);
    state.notice.set(None);
    let _account_user_id = account_user_id.to_string();
    deleting.set(false);
    state.notice.set(Some("Account deleted.".to_string()));
    spawn_account_reload(state);
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use leptos::prelude::*;

    use super::*;

    fn sample_account(user_id: &str, is_admin: bool) -> LocalAccount {
        LocalAccount {
            user_id: user_id.to_string(),
            username: user_id.to_string(),
            is_admin,
            created_at: Utc.with_ymd_and_hms(2026, 4, 17, 1, 0, 0).unwrap(),
        }
    }

    #[test]
    fn password_update_ignores_blank_passwords() {
        assert_eq!(password_update("   ".to_string()), None);
        assert_eq!(
            password_update("password123".to_string()),
            Some("password123".to_string())
        );
    }

    #[test]
    fn account_role_kind_prefers_signed_in_over_admin() {
        let admin = sample_account("admin", true);
        let member = sample_account("member", false);

        assert_eq!(account_role_kind(&admin, "admin"), AccountRoleKind::Active);
        assert_eq!(account_role_kind(&admin, "other"), AccountRoleKind::Admin);
        assert_eq!(account_role_kind(&member, "other"), AccountRoleKind::Member);
        assert_eq!(account_role_label(AccountRoleKind::Active), "signed in");
        assert_eq!(account_role_label(AccountRoleKind::Member), "member");
        assert_eq!(account_role_badge_modifier(AccountRoleKind::Admin), "admin");
    }

    #[test]
    fn account_constraint_label_explains_protected_rows() {
        assert_eq!(
            account_constraint_label(Some(AccountConstraintReason::CurrentUser)),
            "Signed in on this browser"
        );
        assert_eq!(
            account_constraint_label(Some(AccountConstraintReason::LastAdmin)),
            "One admin account must remain"
        );
        assert_eq!(account_constraint_label(None), "");
    }

    #[test]
    fn account_created_label_uses_utc_stamp() {
        assert_eq!(
            account_created_label(&sample_account("member", false)),
            "2026-04-17 01:00 UTC"
        );
    }

    #[test]
    fn account_row_hint_explains_row_state() {
        assert_eq!(
            account_row_hint(false, false, false, true, String::new()),
            "No pending changes"
        );
        assert_eq!(
            account_row_hint(
                false,
                false,
                true,
                false,
                "One admin account must remain".to_string()
            ),
            "One admin account must remain"
        );
        assert_eq!(
            account_row_hint(false, false, true, true, String::new()),
            "Ready to apply"
        );
    }

    #[test]
    fn save_and_delete_button_labels_toggle_with_in_progress_state() {
        assert_eq!(save_button_label(false), "Save");
        assert_eq!(save_button_label(true), "Saving…");
        assert_eq!(delete_button_label(false), "Delete");
        assert_eq!(delete_button_label(true), "Deleting…");
    }

    #[test]
    fn admin_access_label_reflects_admin_flag() {
        assert_eq!(admin_access_label(true), "Enabled");
        assert_eq!(admin_access_label(false), "Standard");
    }

    #[test]
    fn account_count_label_handles_zero_one_and_many() {
        assert_eq!(account_count_label(0), "0 accounts");
        assert_eq!(account_count_label(1), "1 account");
        assert_eq!(account_count_label(5), "5 accounts");
    }

    #[test]
    fn account_sections_and_rows_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            state.current_user_id.set("admin".to_string());
            state.accounts.set(vec![
                sample_account("admin", true),
                sample_account("member", false),
            ]);
            state.loading_accounts.set(false);

            let _ = view! { <CurrentAccountsSection state=state /> };
            let _ = view! { <AccountRow account=sample_account("member", false) state=state /> };
        });
    }

    #[test]
    fn account_row_subcomponents_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let password = RwSignal::new(String::new());
            let admin_checked = RwSignal::new(true);
            let saving = RwSignal::new(false);
            let deleting = RwSignal::new(false);
            let row_dirty = Signal::derive(|| true);
            let can_modify = Signal::derive(|| true);
            let role_label = Signal::derive(|| "admin".to_string());
            let role_badge_class =
                Signal::derive(|| "account-role-pill account-role-pill--admin".to_string());
            let constraint_label = Signal::derive(String::new);

            let _ = view! {
                <AccountRowSummary username="admin".to_string() constraint_label=constraint_label />
            };
            let _ = view! {
                <AccountStateCell
                    role_label=role_label
                    role_badge_class=role_badge_class
                />
            };
            let _ = view! { <AccountPasswordField password=password username="admin".to_string() /> };
            let _ = view! { <AccountAdminToggle admin_checked=admin_checked can_modify=can_modify /> };
            let _ = view! {
                <AccountRowActions
                    saving=saving
                    deleting=deleting
                    row_dirty=row_dirty
                    can_modify=can_modify
                    save_account=Callback::new(|()| {})
                    delete_account=Callback::new(|_: web_sys::MouseEvent| {})
                    constraint_label=constraint_label
                />
            };
        });
    }

    #[test]
    fn accounts_page_content_builds_waiting_and_loading_states() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let _ = view! { <CurrentAccountsSection state=state /> };
        });
    }

    #[test]
    fn account_row_hint_signal_tracks_signal_inputs() {
        let owner = Owner::new();
        owner.with(|| {
            let saving = RwSignal::new(false);
            let deleting = RwSignal::new(false);
            let row_dirty_value = RwSignal::new(false);
            let can_modify_value = RwSignal::new(true);
            let constraint = RwSignal::new(String::new());
            let row_dirty = Signal::derive(move || row_dirty_value.get());
            let can_modify = Signal::derive(move || can_modify_value.get());
            let constraint_label = Signal::derive(move || constraint.get());
            let hint =
                account_row_hint_signal(saving, deleting, row_dirty, can_modify, constraint_label);

            assert_eq!(hint.get(), "No pending changes");
            row_dirty_value.set(true);
            can_modify_value.set(false);
            constraint.set("One admin account must remain".to_string());
            assert_eq!(hint.get(), "One admin account must remain");
            saving.set(true);
            assert_eq!(hint.get(), "Saving changes…");
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn account_save_and_delete_host_update_notice() {
        let owner = Owner::new();
        owner.with(|| {
            let state = AccountsPageState::new();
            let password = RwSignal::new("next-password".to_string());
            let admin_checked = RwSignal::new(true);
            let saving = RwSignal::new(false);
            account_save_host("alice", state, password, admin_checked, saving);
            assert!(!saving.get());
            assert!(password.get().is_empty());
            assert_eq!(state.notice.get(), Some("Account updated.".to_string()));

            let deleting = RwSignal::new(false);
            account_delete_host("alice", state, deleting);
            assert!(!deleting.get());
            assert_eq!(state.notice.get(), Some("Account deleted.".to_string()));
        });
    }
}
