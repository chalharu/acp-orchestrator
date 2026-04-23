//! Composer (message input + submit) component.

use acp_contracts_slash::CompletionCandidate;
use leptos::{html as leptos_html, prelude::*};
#[cfg(target_family = "wasm")]
use wasm_bindgen::{JsCast, closure::Closure};

const MAX_SLASH_PALETTE_ITEMS: usize = 5;
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlashPaletteState {
    Empty,
    Ready(Vec<(usize, CompletionCandidate)>),
}

#[cfg(target_family = "wasm")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerKeyAction {
    Submit,
    SlashSelectNext,
    SlashSelectPrevious,
    SlashApplySelected,
    SlashDismiss,
}

#[derive(Clone)]
struct SubmitDraftContext {
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    form: NodeRef<leptos_html::Form>,
    textarea: NodeRef<leptos_html::Textarea>,
    disabled: Signal<bool>,
    on_submit: Callback<String>,
    restore_focus_after_submit: RwSignal<bool>,
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
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub(crate) apply_selected: Signal<bool>,
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub(crate) on_select_next: Callback<()>,
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub(crate) on_select_previous: Callback<()>,
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub(crate) on_apply_selected: Callback<()>,
    pub(crate) on_apply_index: Callback<usize>,
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
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
    let submit_context = SubmitDraftContext {
        form,
        textarea,
        disabled: controls.disabled,
        on_submit,
        restore_focus_after_submit,
    };
    let handle_submit_context = submit_context.clone();
    let handle_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        submit_draft(draft, handle_submit_context.clone());
    };

    view! {
        <form
            class="panel composer"
            autocomplete="off"
            node_ref=form
            on:submit=handle_submit
        >
            {composer_panel_body(draft, slash, submit_context, controls)}
        </form>
    }
}

fn composer_panel_body(
    draft: RwSignal<String>,
    slash: ComposerSlashProps,
    submit_context: SubmitDraftContext,
    controls: ComposerControls,
) -> impl IntoView {
    view! {
        <ComposerEditor
            draft=draft
            slash=slash
            submit_context=submit_context
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
    slash: ComposerSlashProps,
    submit_context: SubmitDraftContext,
) -> impl IntoView {
    view! {
        <div class="composer__editor">
            <ComposerInput
                draft=draft
                slash=slash
                submit_context=submit_context
            />
            <SlashPalette
                slash_visible=slash.visible
                slash_candidates=slash.candidates
                slash_selected_index=slash.selected_index
                on_apply_index=slash.on_apply_index
            />
        </div>
    }
}

const SLASH_PALETTE_LISTBOX_ID: &str = "slash-palette-listbox";

fn slash_option_id(index: usize) -> String {
    format!("slash-option-{index}")
}

#[component]
fn ComposerInput(
    draft: RwSignal<String>,
    slash: ComposerSlashProps,
    submit_context: SubmitDraftContext,
) -> impl IntoView {
    bind_submit_focus(submit_context.clone());
    composer_input_view(draft, slash, submit_context)
}

#[cfg(target_family = "wasm")]
fn composer_input_view(
    draft: RwSignal<String>,
    slash: ComposerSlashProps,
    submit_context: SubmitDraftContext,
) -> impl IntoView {
    let keydown_submit_context = submit_context.clone();
    let textarea = submit_context.textarea;
    let disabled = submit_context.disabled;
    let slash_keydown_signals = (slash.visible, slash.apply_selected);
    let slash_keydown_callbacks = (
        slash.on_select_next,
        slash.on_select_previous,
        slash.on_apply_selected,
        slash.on_dismiss,
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
            aria-expanded=move || if slash.visible.get() { "true" } else { "false" }
            aria-activedescendant=move || composer_active_descendant(slash.visible.get(), slash.candidates.get().len(), slash.selected_index.get())
            node_ref=textarea
            placeholder="Write a prompt or type / for commands."
            prop:value=move || draft.get()
            on:input=move |ev| update_draft(draft, ev)
            on:keydown=move |ev| handle_composer_keydown(ev, draft, keydown_submit_context.clone(), slash_keydown_signals, slash_keydown_callbacks)
            prop:disabled=move || disabled.get()
        />
    }
}

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

fn bind_submit_focus(submit_context: SubmitDraftContext) {
    #[cfg(not(target_family = "wasm"))]
    {
        let _ = submit_context;
    }

    #[cfg(target_family = "wasm")]
    {
        bind_focus_restore_cancel(
            submit_context.form,
            submit_context.restore_focus_after_submit,
        );
        restore_submit_focus_when_ready(submit_context);
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
fn restore_submit_focus_when_ready(submit_context: SubmitDraftContext) {
    Effect::new(move |_| {
        if !submit_context.restore_focus_after_submit.get() || submit_context.disabled.get() {
            return;
        }

        if let Some(textarea) = submit_context.textarea.get() {
            let _ = textarea.focus();
            submit_context.restore_focus_after_submit.set(false);
        }
    });
}

#[cfg(target_family = "wasm")]
fn handle_composer_keydown(
    ev: web_sys::KeyboardEvent,
    draft: RwSignal<String>,
    submit_context: SubmitDraftContext,
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
        if apply_composer_key_action(action, draft, submit_context, slash_callbacks) {
            ev.prevent_default();
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn composer_input_view(
    draft: RwSignal<String>,
    _slash: ComposerSlashProps,
    submit_context: SubmitDraftContext,
) -> impl IntoView {
    let textarea = submit_context.textarea;
    let disabled = submit_context.disabled;

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
    submit_context: SubmitDraftContext,
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
        ComposerKeyAction::Submit => submit_draft(draft, submit_context),
        ComposerKeyAction::SlashDismiss => on_slash_dismiss.run(()),
    }
    true
}

fn submit_draft(draft: RwSignal<String>, submit_context: SubmitDraftContext) {
    let signal_value = draft.get_untracked();
    let current_value = current_submit_value(
        submit_context
            .textarea
            .get()
            .map(|textarea| textarea.value()),
        signal_value.clone(),
    );

    if current_value != signal_value {
        draft.set(current_value.clone());
    }

    let Some(text) = submit_text(current_value, submit_context.disabled.get_untracked()) else {
        return;
    };
    submit_context.on_submit.run(text);
    submit_context.restore_focus_after_submit.set(true);
}

fn current_submit_value(live_value: Option<String>, draft_value: String) -> String {
    live_value.unwrap_or(draft_value)
}

fn submit_text(current_value: String, disabled: bool) -> Option<String> {
    let text = current_value.trim().to_string();
    (!disabled && !text.is_empty()).then_some(text)
}

#[component]
fn SlashPalette(
    #[prop(into)] slash_visible: Signal<bool>,
    #[prop(into)] slash_candidates: Signal<Vec<CompletionCandidate>>,
    #[prop(into)] slash_selected_index: Signal<usize>,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    #[rustfmt::skip]
    view! {
        <Show when=move || should_render_slash_palette(slash_visible)>
            <section class="composer__slash-palette" aria-label="Slash command suggestions">{move || render_slash_palette_state(slash_palette_state(slash_candidates), slash_selected_index.get(), on_apply_index)}</section>
        </Show>
    }
}

fn should_render_slash_palette(slash_visible: Signal<bool>) -> bool {
    slash_visible.get()
}

fn slash_palette_state(slash_candidates: Signal<Vec<CompletionCandidate>>) -> SlashPaletteState {
    let items = slash_candidates
        .get()
        .into_iter()
        .enumerate()
        .take(MAX_SLASH_PALETTE_ITEMS)
        .collect::<Vec<_>>();
    if items.is_empty() {
        SlashPaletteState::Empty
    } else {
        SlashPaletteState::Ready(items)
    }
}

fn render_slash_palette_state(
    state: SlashPaletteState,
    selected_index: usize,
    on_apply_index: Callback<usize>,
) -> AnyView {
    match state {
        SlashPaletteState::Empty => {
            view! { <p class="composer__slash-empty">"No matching slash commands."</p> }.into_any()
        }
        SlashPaletteState::Ready(items) => view! {
            <SlashPaletteList
                items=items
                selected_index=selected_index
                on_apply_index=on_apply_index
            />
        }
        .into_any(),
    }
}

#[component]
fn SlashPaletteList(
    items: Vec<(usize, CompletionCandidate)>,
    selected_index: usize,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    view! {
        <ul id=SLASH_PALETTE_LISTBOX_ID role="listbox" class="composer__slash-list">
            <For
                each=move || items.clone()
                key=|(index, candidate)| (index.to_owned(), candidate.label.clone())
                children=move |(index, candidate)| {
                    view! {
                        <SlashPaletteItem
                            index=index
                            candidate=candidate
                            is_selected=index == selected_index
                            on_apply_index=on_apply_index
                        />
                    }
                }
            />
        </ul>
    }
}

#[component]
fn SlashPaletteItem(
    index: usize,
    candidate: CompletionCandidate,
    is_selected: bool,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    let CompletionCandidate { label, detail, .. } = candidate;
    let option_id = slash_option_id(index);

    view! {
        <li
            id=option_id
            role="option"
            aria-selected=if is_selected { "true" } else { "false" }
        >
            <button
                type="button"
                class=if is_selected {
                    "composer__slash-item composer__slash-item--selected"
                } else {
                    "composer__slash-item"
                }
                tabindex="-1"
                on:mousedown=move |ev| {
                    ev.prevent_default();
                    on_apply_index.run(index);
                }
                on:keydown=move |ev| {
                    if matches!(ev.key().as_str(), "Enter" | " ") {
                        ev.prevent_default();
                        on_apply_index.run(index);
                    }
                }
            >
                <span class="composer__slash-label">{label}</span>
                <span class="composer__slash-detail">{detail}</span>
            </button>
        </li>
    }
}

#[component]
fn ComposerFooter(
    #[prop(into)] status_text: Signal<String>,
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="composer__footer">
            <p class="composer__status" hidden=move || status_text.get().is_empty()>
                {move || status_text.get()}
            </p>
            <ComposerActions
                disabled=disabled
                show_cancel=show_cancel
                cancel_disabled=cancel_disabled
                on_cancel=on_cancel
            />
        </div>
    }
}

#[component]
fn ComposerActions(
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="composer__actions">
            <Show when=move || show_cancel.get()>
                <button
                    class="pending-list__button--secondary composer__cancel"
                    type="button"
                    on:click=move |_| on_cancel.run(())
                    prop:disabled=move || cancel_disabled.get()
                >
                    "Cancel"
                </button>
            </Show>
            <button
                class="composer__submit"
                type="submit"
                prop:disabled=move || disabled.get()
            >
                "Send"
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use acp_contracts_slash::{CompletionCandidate, CompletionKind};
    use leptos::prelude::*;

    use super::{
        Composer, ComposerActions, ComposerControls, ComposerFooter, ComposerSlashProps,
        SlashPalette, SlashPaletteItem, SlashPaletteList, SlashPaletteState, SlashTestCallbacks,
        SlashTestSignals, composer_active_descendant, current_submit_value,
        render_slash_palette_state, should_render_slash_palette, slash_option_id,
        slash_palette_state, submit_text,
    };
    #[cfg(target_family = "wasm")]
    use super::{
        ComposerKeyAction, SubmitDraftContext, apply_composer_key_action, composer_key_action,
    };

    fn make_candidate(label: &str) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            insert_text: label.to_string(),
            detail: "detail".to_string(),
            kind: CompletionKind::Command,
        }
    }

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

    #[test]
    fn should_render_slash_palette_follows_visible_signal() {
        let owner = Owner::new();
        owner.with(|| {
            let (visible, ..) = make_slash_signals(true, vec![], 0, false);
            assert!(should_render_slash_palette(visible));

            let (hidden, ..) = make_slash_signals(false, vec![], 0, false);
            assert!(!should_render_slash_palette(hidden));
        });
    }

    #[test]
    fn slash_palette_state_empty_when_no_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let (_, candidates, ..) = make_slash_signals(true, vec![], 0, false);
            assert_eq!(slash_palette_state(candidates), SlashPaletteState::Empty);
        });
    }

    #[test]
    fn slash_palette_state_ready_with_indexed_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let candidates = vec![make_candidate("/help"), make_candidate("/clear")];
            let (_, candidates, ..) = make_slash_signals(true, candidates, 0, false);
            let state = slash_palette_state(candidates);
            assert!(matches!(
                state,
                SlashPaletteState::Ready(items)
                    if items.len() == 2 && items[0].0 == 0 && items[1].0 == 1
            ));
        });
    }

    #[test]
    fn slash_palette_state_caps_at_max_items() {
        let owner = Owner::new();
        owner.with(|| {
            let candidates: Vec<_> = (0..10)
                .map(|i| make_candidate(&format!("/cmd{i}")))
                .collect();
            let (_, candidates, ..) = make_slash_signals(true, candidates, 0, false);
            let state = slash_palette_state(candidates);
            assert!(matches!(state, SlashPaletteState::Ready(items) if items.len() == 5));
        });
    }

    #[test]
    fn render_slash_palette_state_handles_empty_and_ready_states() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = render_slash_palette_state(
                SlashPaletteState::Empty,
                0,
                Callback::new(|_: usize| {}),
            );
            let _ = render_slash_palette_state(
                SlashPaletteState::Ready(vec![(0, make_candidate("/help"))]),
                0,
                Callback::new(|_: usize| {}),
            );
        });
    }

    #[test]
    fn slash_palette_components_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let candidates = vec![make_candidate("/help"), make_candidate("/clear")];
            let on_apply = Callback::new(|_: usize| {});
            let (visible, candidates_signal, selected_index, _) =
                make_slash_signals(true, candidates.clone(), 1, true);

            let _ = view! {
                <SlashPalette
                    slash_visible=visible
                    slash_candidates=candidates_signal
                    slash_selected_index=selected_index
                    on_apply_index=on_apply
                />
            };
            let _ = view! {
                <SlashPaletteList
                    items=vec![
                        (0, make_candidate("/help")),
                        (1, make_candidate("/clear")),
                    ]
                    selected_index=1
                    on_apply_index=on_apply
                />
            };
            let _ = view! {
                <SlashPaletteItem
                    index=0
                    candidate=make_candidate("/help")
                    is_selected=true
                    on_apply_index=on_apply
                />
            };
        });
    }

    #[test]
    fn composer_footer_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <ComposerFooter
                    status_text=Signal::derive(|| "Ready".to_string())
                    disabled=Signal::derive(|| false)
                    show_cancel=Signal::derive(|| true)
                    cancel_disabled=Signal::derive(|| false)
                    on_cancel=Callback::new(|()| {})
                />
            };
        });
    }

    #[test]
    fn composer_actions_build_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <ComposerActions
                    disabled=Signal::derive(|| false)
                    show_cancel=Signal::derive(|| true)
                    cancel_disabled=Signal::derive(|| false)
                    on_cancel=Callback::new(|()| {})
                />
            };
            // Cover the show_cancel=false path (Show fallback).
            let _ = view! {
                <ComposerActions
                    disabled=Signal::derive(|| false)
                    show_cancel=Signal::derive(|| false)
                    cancel_disabled=Signal::derive(|| false)
                    on_cancel=Callback::new(|()| {})
                />
            };
        });
    }

    // -----------------------------------------------------------------------
    // current_submit_value – None live value path
    // -----------------------------------------------------------------------

    #[test]
    fn current_submit_value_falls_back_to_draft_when_live_value_is_absent() {
        assert_eq!(
            current_submit_value(None, "draft-text".to_string()),
            "draft-text"
        );
    }

    // -----------------------------------------------------------------------
    // Composer – full component build (covers the constructor function body)
    // -----------------------------------------------------------------------

    fn make_slash_callbacks() -> SlashTestCallbacks {
        (
            Callback::new(|()| {}),
            Callback::new(|()| {}),
            Callback::new(|()| {}),
            Callback::new(|_: usize| {}),
            Callback::new(|()| {}),
        )
    }

    fn make_slash_props(
        visible: Signal<bool>,
        candidates: Signal<Vec<CompletionCandidate>>,
        selected_index: Signal<usize>,
        apply_selected: Signal<bool>,
        callbacks: SlashTestCallbacks,
    ) -> ComposerSlashProps {
        ComposerSlashProps {
            visible,
            candidates,
            selected_index,
            apply_selected,
            on_select_next: callbacks.0,
            on_select_previous: callbacks.1,
            on_apply_selected: callbacks.2,
            on_apply_index: callbacks.3,
            on_dismiss: callbacks.4,
        }
    }

    #[cfg(target_family = "wasm")]
    fn submit_context(
        disabled: Signal<bool>,
        submitted: RwSignal<Vec<String>>,
        restore_focus_after_submit: RwSignal<bool>,
    ) -> SubmitDraftContext {
        SubmitDraftContext {
            form: NodeRef::new(),
            textarea: NodeRef::new(),
            disabled,
            on_submit: Callback::new(move |text: String| {
                submitted.update(|items| items.push(text));
            }),
            restore_focus_after_submit,
        }
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
                    slash=make_slash_props(
                        slash_visible,
                        slash_candidates,
                        slash_selected_index,
                        slash_apply_selected,
                        (
                            on_slash_select_next,
                            on_slash_select_previous,
                            on_slash_apply_selected,
                            on_slash_apply_index,
                            on_slash_dismiss,
                        ),
                    )
                />
            };
        });
    }

    #[cfg(target_family = "wasm")]
    #[test]
    fn apply_composer_key_action_submits_trimmed_drafts() {
        let owner = Owner::new();
        owner.with(|| {
            let submitted = RwSignal::new(Vec::<String>::new());
            let restore_focus_after_submit = RwSignal::new(false);
            let submit_context = submit_context(
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
                submit_context,
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
            let submit_context = submit_context(
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
                submit_context.clone(),
                callbacks,
            );
            apply_composer_key_action(
                ComposerKeyAction::SlashSelectPrevious,
                draft,
                submit_context.clone(),
                callbacks,
            );
            apply_composer_key_action(
                ComposerKeyAction::SlashApplySelected,
                draft,
                submit_context.clone(),
                callbacks,
            );
            apply_composer_key_action(
                ComposerKeyAction::SlashDismiss,
                draft,
                submit_context,
                callbacks,
            );
            assert_eq!(next_calls.get(), 1);
            assert_eq!(prev_calls.get(), 1);
            assert_eq!(apply_calls.get(), 1);
            assert_eq!(dismiss_calls.get(), 1);
        });
    }
}
