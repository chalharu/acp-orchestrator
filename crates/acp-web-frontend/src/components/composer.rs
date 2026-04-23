//! Composer (message input + submit) component.

use acp_contracts_slash::CompletionCandidate;
use leptos::{html as leptos_html, prelude::*};
#[cfg(target_family = "wasm")]
use wasm_bindgen::{JsCast, closure::Closure};

use super::composer_footer::ComposerFooter;
#[cfg(target_family = "wasm")]
use super::composer_palette::SLASH_PALETTE_LISTBOX_ID;
use super::composer_palette::{SlashPalette, slash_option_id};

#[cfg(target_family = "wasm")]
type SlashKeydownSignals = (Signal<bool>, Signal<bool>);
#[cfg(target_family = "wasm")]
type SlashKeydownCallbacks = (Callback<()>, Callback<()>, Callback<()>, Callback<()>);
#[cfg(test)]
type SlashTestSignals = (
    Signal<bool>,
    Signal<Vec<CompletionCandidate>>,
    Signal<usize>,
    Signal<bool>,
);
#[cfg(test)]
type SlashTestCallbacks = (
    Callback<()>,
    Callback<()>,
    Callback<()>,
    Callback<usize>,
    Callback<()>,
);
type ComposerSlashSignals = (
    Signal<bool>,
    Signal<Vec<CompletionCandidate>>,
    Signal<usize>,
    Signal<bool>,
);
type ComposerSlashCallbacks = (
    Callback<()>,
    Callback<()>,
    Callback<()>,
    Callback<usize>,
    Callback<()>,
);
type SubmitDraftRuntime = (
    NodeRef<leptos_html::Form>,
    NodeRef<leptos_html::Textarea>,
    Signal<bool>,
    Callback<String>,
    RwSignal<bool>,
);

#[cfg(target_family = "wasm")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerKeyAction {
    Submit,
    SlashSelectNext,
    SlashSelectPrevious,
    SlashApplySelected,
    SlashDismiss,
}

#[derive(Clone, Copy)]
pub(crate) struct ComposerControls {
    pub(crate) disabled: Signal<bool>,
    pub(crate) status_text: Signal<String>,
    pub(crate) show_cancel: Signal<bool>,
    pub(crate) cancel_disabled: Signal<bool>,
    pub(crate) on_cancel: Callback<()>,
}

#[derive(Clone, Copy)]
pub(crate) struct ComposerSlashProps {
    pub(crate) visible: Signal<bool>,
    pub(crate) candidates: Signal<Vec<CompletionCandidate>>,
    pub(crate) selected_index: Signal<usize>,
    pub(crate) apply_selected: Signal<bool>,
    pub(crate) on_select_next: Callback<()>,
    pub(crate) on_select_previous: Callback<()>,
    pub(crate) on_apply_selected: Callback<()>,
    pub(crate) on_apply_index: Callback<usize>,
    pub(crate) on_dismiss: Callback<()>,
}

#[component]
pub(crate) fn Composer(
    draft: RwSignal<String>,
    on_submit: Callback<String>,
    controls: ComposerControls,
    slash: ComposerSlashProps,
) -> impl IntoView {
    let form = NodeRef::<leptos_html::Form>::new();
    let textarea = NodeRef::<leptos_html::Textarea>::new();
    let restore_focus_after_submit = RwSignal::new(false);
    let submit_runtime = (
        form,
        textarea,
        controls.disabled,
        on_submit,
        restore_focus_after_submit,
    );
    let slash_signals = (
        slash.visible,
        slash.candidates,
        slash.selected_index,
        slash.apply_selected,
    );
    let slash_callbacks = (
        slash.on_select_next,
        slash.on_select_previous,
        slash.on_apply_selected,
        slash.on_apply_index,
        slash.on_dismiss,
    );
    let handle_submit_runtime = submit_runtime;
    let handle_submit = composer_submit_handler(draft, handle_submit_runtime);
    let (form, _, _, _, _) = submit_runtime;

    view! {
        <form
            class="panel composer"
            autocomplete="off"
            node_ref=form
            on:submit=handle_submit
        >
            {composer_panel_body(draft, slash_signals, slash_callbacks, submit_runtime, controls)}
        </form>
    }
}

fn submit_handler<E>(
    draft: RwSignal<String>,
    submit_runtime: SubmitDraftRuntime,
) -> impl Fn(E) + Copy + 'static
where
    E: 'static,
{
    move |_ev: E| submit_draft(draft, submit_runtime)
}

#[cfg(target_family = "wasm")]
fn composer_submit_handler(
    draft: RwSignal<String>,
    submit_runtime: SubmitDraftRuntime,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        submit_draft(draft, submit_runtime);
    }
}

#[cfg(not(target_family = "wasm"))]
fn composer_submit_handler(
    draft: RwSignal<String>,
    submit_runtime: SubmitDraftRuntime,
) -> impl Fn(web_sys::SubmitEvent) + Copy + 'static {
    submit_handler(draft, submit_runtime)
}

fn composer_panel_body(
    draft: RwSignal<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
    submit_runtime: SubmitDraftRuntime,
    controls: ComposerControls,
) -> impl IntoView {
    view! {
        <ComposerEditor
            draft=draft
            slash_signals=slash_signals
            slash_callbacks=slash_callbacks
            submit_runtime=submit_runtime
        />
        <ComposerFooter
            status_text=controls.status_text
            disabled=controls.disabled
            show_cancel=controls.show_cancel
            cancel_disabled=controls.cancel_disabled
            on_cancel=controls.on_cancel
        />
    }
}

#[component]
fn ComposerEditor(
    draft: RwSignal<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
    submit_runtime: SubmitDraftRuntime,
) -> impl IntoView {
    let (slash_visible, slash_candidates, slash_selected_index, _) = slash_signals;
    let (_, _, _, on_apply_index, _) = slash_callbacks;

    view! {
        <div class="composer__editor">
            <ComposerInput
                draft=draft
                slash_signals=slash_signals
                slash_callbacks=slash_callbacks
                submit_runtime=submit_runtime
            />
            <SlashPalette
                slash_visible=slash_visible
                slash_candidates=slash_candidates
                slash_selected_index=slash_selected_index
                on_apply_index=on_apply_index
            />
        </div>
    }
}

#[component]
fn ComposerInput(
    draft: RwSignal<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
    submit_runtime: SubmitDraftRuntime,
) -> impl IntoView {
    bind_submit_focus(submit_runtime);
    composer_input_view(draft, slash_signals, slash_callbacks, submit_runtime)
}

#[cfg(target_family = "wasm")]
fn composer_input_view(
    draft: RwSignal<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
    submit_runtime: SubmitDraftRuntime,
) -> impl IntoView {
    let keydown_submit_runtime = submit_runtime.clone();
    let (_, textarea, disabled, _, _) = submit_runtime;
    let (slash_visible, slash_candidates, slash_selected_index, slash_apply_selected) =
        slash_signals;
    let slash_keydown_callbacks = (
        slash_callbacks.0,
        slash_callbacks.1,
        slash_callbacks.2,
        slash_callbacks.4,
    );

    #[rustfmt::skip]
    view! {
        <label class="sr-only" for="composer-input">"Prompt"</label>
        <textarea
            id="composer-input"
            name="prompt"
            rows="4"
            role="combobox"
            aria-autocomplete="list"
            aria-haspopup="listbox"
            aria-controls=SLASH_PALETTE_LISTBOX_ID
            aria-expanded=move || if slash_visible.get() { "true" } else { "false" }
            aria-activedescendant=move || composer_active_descendant(slash_visible.get(), slash_candidates.get().len(), slash_selected_index.get())
            node_ref=textarea
            placeholder="Write a prompt or type / for commands."
            prop:value=move || draft.get()
            on:input=move |ev| update_draft(draft, ev)
            on:keydown=move |ev| handle_composer_keydown(ev, draft, keydown_submit_runtime.clone(), (slash_visible, slash_apply_selected), slash_keydown_callbacks)
            prop:disabled=move || disabled.get()
        />
    }
}

#[allow(dead_code)]
fn composer_active_descendant(
    slash_visible: bool,
    candidate_count: usize,
    selected_index: usize,
) -> Option<String> {
    if slash_visible && candidate_count > 0 {
        Some(slash_option_id(selected_index))
    } else {
        None
    }
}

fn update_draft(draft: RwSignal<String>, ev: web_sys::Event) {
    draft.set(event_target_value(&ev));
}

fn bind_submit_focus(submit_runtime: SubmitDraftRuntime) {
    #[cfg(not(target_family = "wasm"))]
    {
        let _ = submit_runtime;
    }

    #[cfg(target_family = "wasm")]
    {
        let (form, _, _, _, restore_focus_after_submit) = submit_runtime.clone();
        bind_focus_restore_cancel(form, restore_focus_after_submit);
        restore_submit_focus_when_ready(submit_runtime);
    }
}

#[cfg(target_family = "wasm")]
fn bind_focus_restore_cancel(form: NodeRef<leptos_html::Form>, restore: RwSignal<bool>) {
    Effect::new(move |_| {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let Some(form) = form.get() else {
            return;
        };
        let form_node = form.unchecked_into::<web_sys::Node>();
        attach_pointer_restore_cancel_listener(&document, &form_node, restore);
        attach_focus_restore_cancel_listener(&document, &form_node, restore);
    });
}

#[cfg(target_family = "wasm")]
fn attach_pointer_restore_cancel_listener(
    document: &web_sys::Document,
    form_node: &web_sys::Node,
    restore: RwSignal<bool>,
) {
    let form_node = form_node.clone();
    let listener = Closure::wrap(Box::new(move |ev: web_sys::PointerEvent| {
        clear_restore_when_target_leaves_form(ev.target(), &form_node, restore);
    }) as Box<dyn FnMut(web_sys::PointerEvent)>);
    let _ =
        document.add_event_listener_with_callback("pointerdown", listener.as_ref().unchecked_ref());
    listener.forget();
}

#[cfg(target_family = "wasm")]
fn attach_focus_restore_cancel_listener(
    document: &web_sys::Document,
    form_node: &web_sys::Node,
    restore: RwSignal<bool>,
) {
    let form_node = form_node.clone();
    let listener = Closure::wrap(Box::new(move |ev: web_sys::FocusEvent| {
        clear_restore_when_target_leaves_form(ev.target(), &form_node, restore);
    }) as Box<dyn FnMut(web_sys::FocusEvent)>);
    let _ = document.add_event_listener_with_callback("focusin", listener.as_ref().unchecked_ref());
    listener.forget();
}

#[cfg(target_family = "wasm")]
fn clear_restore_when_target_leaves_form(
    target: Option<web_sys::EventTarget>,
    form_node: &web_sys::Node,
    restore: RwSignal<bool>,
) {
    if !restore.get_untracked() {
        return;
    }
    let Some(target_node) = target
        .as_ref()
        .and_then(|target| target.dyn_ref::<web_sys::Node>())
    else {
        restore.set(false);
        return;
    };
    if !form_node.contains(Some(target_node)) {
        restore.set(false);
    }
}

#[cfg(target_family = "wasm")]
fn restore_submit_focus_when_ready(submit_runtime: SubmitDraftRuntime) {
    Effect::new(move |_| {
        let (_, textarea, disabled, _, restore_focus_after_submit) = submit_runtime.clone();
        if !restore_focus_after_submit.get() || disabled.get() {
            return;
        }

        if let Some(textarea) = textarea.get() {
            let _ = textarea.focus();
            restore_focus_after_submit.set(false);
        }
    });
}

#[cfg(target_family = "wasm")]
fn handle_composer_keydown(
    ev: web_sys::KeyboardEvent,
    draft: RwSignal<String>,
    submit_runtime: SubmitDraftRuntime,
    slash_signals: SlashKeydownSignals,
    slash_callbacks: SlashKeydownCallbacks,
) {
    if let Some(action) = composer_key_action(
        &ev.key(),
        ev.is_composing(),
        ev.shift_key(),
        slash_signals.0.get_untracked(),
        slash_signals.1.get_untracked(),
    ) {
        if apply_composer_key_action(action, draft, submit_runtime, slash_callbacks) {
            ev.prevent_default();
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn composer_input_view(
    draft: RwSignal<String>,
    _slash_signals: ComposerSlashSignals,
    _slash_callbacks: ComposerSlashCallbacks,
    submit_runtime: SubmitDraftRuntime,
) -> impl IntoView {
    let (_, textarea, disabled, _, _) = submit_runtime;

    view! {
        <label class="sr-only" for="composer-input">"Prompt"</label>
        <textarea
            id="composer-input"
            name="prompt"
            rows="4"
            node_ref=textarea
            placeholder="Write a prompt or type / for commands."
            prop:value=move || draft.get()
            on:input=move |ev| update_draft(draft, ev)
            prop:disabled=move || disabled.get()
        />
    }
}

#[cfg(target_family = "wasm")]
#[rustfmt::skip]
fn composer_key_action(
    key: &str,
    is_composing: bool,
    shift_key: bool,
    slash_visible: bool,
    slash_apply_selected: bool,
) -> Option<ComposerKeyAction> {
    if is_composing { return None; }
    if slash_visible {
        return match key {
            "ArrowDown" => Some(ComposerKeyAction::SlashSelectNext),
            "ArrowUp" => Some(ComposerKeyAction::SlashSelectPrevious),
            "Tab" if !shift_key && slash_apply_selected => Some(ComposerKeyAction::SlashApplySelected),
            "Enter" if !shift_key && slash_apply_selected => Some(ComposerKeyAction::SlashApplySelected),
            "Enter" if !shift_key => Some(ComposerKeyAction::Submit),
            "Escape" => Some(ComposerKeyAction::SlashDismiss),
            _ => None,
        };
    }
    (key == "Enter" && !shift_key).then_some(ComposerKeyAction::Submit)
}

#[cfg(target_family = "wasm")]
fn apply_composer_key_action(
    action: ComposerKeyAction,
    draft: RwSignal<String>,
    submit_runtime: SubmitDraftRuntime,
    (
        on_slash_select_next,
        on_slash_select_previous,
        on_slash_apply_selected,
        on_slash_dismiss,
    ): SlashKeydownCallbacks,
) -> bool {
    match action {
        ComposerKeyAction::SlashSelectNext => on_slash_select_next.run(()),
        ComposerKeyAction::SlashSelectPrevious => on_slash_select_previous.run(()),
        ComposerKeyAction::SlashApplySelected => on_slash_apply_selected.run(()),
        ComposerKeyAction::Submit => submit_draft(draft, submit_runtime),
        ComposerKeyAction::SlashDismiss => on_slash_dismiss.run(()),
    }
    true
}

fn submit_draft(draft: RwSignal<String>, submit_runtime: SubmitDraftRuntime) {
    let (_, textarea, disabled, on_submit, restore_focus_after_submit) = submit_runtime;
    let signal_value = draft.get_untracked();
    let current_value = current_submit_value(
        textarea.get().map(|textarea| textarea.value()),
        signal_value.clone(),
    );

    if current_value != signal_value {
        draft.set(current_value.clone());
    }

    let Some(text) = submit_text(current_value, disabled.get_untracked()) else {
        return;
    };
    on_submit.run(text);
    restore_focus_after_submit.set(true);
}

fn current_submit_value(live_value: Option<String>, draft_value: String) -> String {
    live_value.unwrap_or(draft_value)
}

fn submit_text(current_value: String, disabled: bool) -> Option<String> {
    let text = current_value.trim().to_string();
    (!disabled && !text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use acp_contracts_slash::CompletionCandidate;
    use leptos::prelude::*;
    #[cfg(not(target_family = "wasm"))]
    use wasm_bindgen::{JsCast, JsValue};

    use super::super::composer_palette::slash_option_id;
    use super::{
        Composer, ComposerControls, ComposerSlashProps, SlashTestCallbacks, SlashTestSignals,
        SubmitDraftRuntime, composer_active_descendant, composer_submit_handler,
        current_submit_value, submit_draft, submit_handler, submit_text,
    };
    #[cfg(target_family = "wasm")]
    use super::{ComposerKeyAction, apply_composer_key_action, composer_key_action};

    fn make_slash_signals(
        visible: bool,
        candidates: Vec<CompletionCandidate>,
        selected_index: usize,
        apply_selected: bool,
    ) -> SlashTestSignals {
        (
            Signal::derive(move || visible),
            Signal::derive(move || candidates.clone()),
            Signal::derive(move || selected_index),
            Signal::derive(move || apply_selected),
        )
    }

    #[test]
    fn current_submit_value_prefers_the_live_textarea_value() {
        assert_eq!(
            current_submit_value(Some("test".to_string()), "/help".to_string()),
            "test"
        );
    }

    #[test]
    fn current_submit_value_falls_back_to_draft_when_live_value_is_absent() {
        assert_eq!(
            current_submit_value(None, "draft-text".to_string()),
            "draft-text"
        );
    }

    #[test]
    fn submit_text_trims_and_blocks_disabled_or_empty_submissions() {
        assert_eq!(
            submit_text("  test  ".to_string(), false),
            Some("test".to_string())
        );
        assert_eq!(submit_text("   ".to_string(), false), None);
        assert_eq!(submit_text("test".to_string(), true), None);
    }

    #[test]
    fn slash_option_id_formats_index_as_string() {
        assert_eq!(slash_option_id(0), "slash-option-0");
        assert_eq!(slash_option_id(3), "slash-option-3");
    }

    #[test]
    fn composer_active_descendant_requires_a_visible_non_empty_palette() {
        assert_eq!(
            composer_active_descendant(true, 2, 1),
            Some("slash-option-1".to_string())
        );
        assert_eq!(composer_active_descendant(false, 2, 1), None);
        assert_eq!(composer_active_descendant(true, 0, 1), None);
    }

    #[cfg(target_family = "wasm")]
    #[test]
    fn composer_key_action_covers_submit_and_slash_navigation_paths() {
        assert_eq!(
            composer_key_action("Enter", false, false, false, false),
            Some(ComposerKeyAction::Submit)
        );
        assert_eq!(
            composer_key_action("ArrowDown", false, false, true, false),
            Some(ComposerKeyAction::SlashSelectNext)
        );
        assert_eq!(
            composer_key_action("ArrowUp", false, false, true, false),
            Some(ComposerKeyAction::SlashSelectPrevious)
        );
        assert_eq!(
            composer_key_action("Tab", false, false, true, true),
            Some(ComposerKeyAction::SlashApplySelected)
        );
        assert_eq!(
            composer_key_action("Enter", false, false, true, true),
            Some(ComposerKeyAction::SlashApplySelected)
        );
        assert_eq!(
            composer_key_action("Escape", false, false, true, false),
            Some(ComposerKeyAction::SlashDismiss)
        );
        assert_eq!(
            composer_key_action("Enter", true, false, false, false),
            None
        );
        assert_eq!(composer_key_action("x", false, false, true, false), None);
    }

    fn make_slash_callbacks() -> SlashTestCallbacks {
        (
            Callback::new(|()| {}),
            Callback::new(|()| {}),
            Callback::new(|()| {}),
            Callback::new(|_: usize| {}),
            Callback::new(|()| {}),
        )
    }

    fn submit_runtime(
        disabled: Signal<bool>,
        submitted: RwSignal<Vec<String>>,
        restore_focus_after_submit: RwSignal<bool>,
    ) -> SubmitDraftRuntime {
        (
            NodeRef::new(),
            NodeRef::new(),
            disabled,
            Callback::new(move |text: String| {
                submitted.update(|items| items.push(text));
            }),
            restore_focus_after_submit,
        )
    }

    struct FakeSubmitEvent;

    #[cfg(not(target_family = "wasm"))]
    fn fake_submit_event() -> web_sys::SubmitEvent {
        JsValue::NULL.unchecked_into()
    }

    #[test]
    fn composer_full_component_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let draft = RwSignal::new(String::new());
            let (slash_visible, slash_candidates, slash_selected_index, slash_apply_selected) =
                make_slash_signals(false, vec![], 0, false);
            let (
                on_slash_select_next,
                on_slash_select_previous,
                on_slash_apply_selected,
                on_slash_apply_index,
                on_slash_dismiss,
            ) = make_slash_callbacks();

            let _ = view! {
                <Composer
                    draft=draft
                    on_submit=Callback::new(|_: String| {})
                    controls=ComposerControls {
                        disabled: Signal::derive(|| false),
                        status_text: Signal::derive(String::new),
                        show_cancel: Signal::derive(|| false),
                        cancel_disabled: Signal::derive(|| false),
                        on_cancel: Callback::new(|()| {}),
                    }
                    slash=ComposerSlashProps {
                        visible: slash_visible,
                        candidates: slash_candidates,
                        selected_index: slash_selected_index,
                        apply_selected: slash_apply_selected,
                        on_select_next: on_slash_select_next,
                        on_select_previous: on_slash_select_previous,
                        on_apply_selected: on_slash_apply_selected,
                        on_apply_index: on_slash_apply_index,
                        on_dismiss: on_slash_dismiss,
                    }
                />
            };
        });
    }

    #[test]
    fn submit_draft_runs_submit_callback_and_marks_focus_restore() {
        let owner = Owner::new();
        owner.with(|| {
            let submitted = RwSignal::new(Vec::<String>::new());
            let restore_focus_after_submit = RwSignal::new(false);
            let runtime = submit_runtime(
                Signal::derive(|| false),
                submitted,
                restore_focus_after_submit,
            );
            let draft = RwSignal::new("  prompt  ".to_string());

            submit_draft(draft, runtime);

            assert_eq!(submitted.get(), vec!["prompt".to_string()]);
            assert!(restore_focus_after_submit.get());
        });
    }

    #[test]
    fn submit_handler_runs_submit_callback() {
        let owner = Owner::new();
        owner.with(|| {
            let submitted = RwSignal::new(Vec::<String>::new());
            let restore_focus_after_submit = RwSignal::new(false);
            let runtime = submit_runtime(
                Signal::derive(|| false),
                submitted,
                restore_focus_after_submit,
            );
            let draft = RwSignal::new("  prompt  ".to_string());

            submit_handler(draft, runtime)(FakeSubmitEvent);

            assert_eq!(submitted.get(), vec!["prompt".to_string()]);
            assert!(restore_focus_after_submit.get());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn composer_submit_handler_submits_with_host_submit_event() {
        let owner = Owner::new();
        owner.with(|| {
            let submitted = RwSignal::new(Vec::<String>::new());
            let restore_focus_after_submit = RwSignal::new(false);
            let runtime = submit_runtime(
                Signal::derive(|| false),
                submitted,
                restore_focus_after_submit,
            );
            let draft = RwSignal::new("  prompt  ".to_string());

            composer_submit_handler(draft, runtime)(fake_submit_event());

            assert_eq!(submitted.get(), vec!["prompt".to_string()]);
            assert!(restore_focus_after_submit.get());
        });
    }

    #[cfg(target_family = "wasm")]
    #[test]
    fn apply_composer_key_action_submits_trimmed_drafts() {
        let owner = Owner::new();
        owner.with(|| {
            let submitted = RwSignal::new(Vec::<String>::new());
            let restore_focus_after_submit = RwSignal::new(false);
            let submit_runtime = submit_runtime(
                Signal::derive(|| false),
                submitted,
                restore_focus_after_submit,
            );
            let draft = RwSignal::new("  prompt  ".to_string());
            let callbacks = (
                Callback::new(|()| {}),
                Callback::new(|()| {}),
                Callback::new(|()| {}),
                Callback::new(|()| {}),
            );

            assert!(apply_composer_key_action(
                ComposerKeyAction::Submit,
                draft,
                submit_runtime,
                callbacks
            ));
            assert_eq!(submitted.get(), vec!["prompt".to_string()]);
            assert!(restore_focus_after_submit.get());
        });
    }

    #[cfg(target_family = "wasm")]
    #[test]
    fn apply_composer_key_action_runs_slash_callbacks() {
        let owner = Owner::new();
        owner.with(|| {
            let submit_runtime = submit_runtime(
                Signal::derive(|| false),
                RwSignal::new(Vec::<String>::new()),
                RwSignal::new(false),
            );
            let draft = RwSignal::new("prompt".to_string());
            let next_calls = RwSignal::new(0usize);
            let prev_calls = RwSignal::new(0usize);
            let apply_calls = RwSignal::new(0usize);
            let dismiss_calls = RwSignal::new(0usize);
            let callbacks = (
                Callback::new(move |()| next_calls.update(|value| *value += 1)),
                Callback::new(move |()| prev_calls.update(|value| *value += 1)),
                Callback::new(move |()| apply_calls.update(|value| *value += 1)),
                Callback::new(move |()| dismiss_calls.update(|value| *value += 1)),
            );

            apply_composer_key_action(
                ComposerKeyAction::SlashSelectNext,
                draft,
                submit_runtime.clone(),
                callbacks,
            );
            apply_composer_key_action(
                ComposerKeyAction::SlashSelectPrevious,
                draft,
                submit_runtime.clone(),
                callbacks,
            );
            apply_composer_key_action(
                ComposerKeyAction::SlashApplySelected,
                draft,
                submit_runtime.clone(),
                callbacks,
            );
            apply_composer_key_action(
                ComposerKeyAction::SlashDismiss,
                draft,
                submit_runtime,
                callbacks,
            );
            assert_eq!(next_calls.get(), 1);
            assert_eq!(prev_calls.get(), 1);
            assert_eq!(apply_calls.get(), 1);
            assert_eq!(dismiss_calls.get(), 1);
        });
    }
}
