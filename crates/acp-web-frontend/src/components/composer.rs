//! Composer (message input + submit) component.

use acp_contracts::CompletionCandidate;
use leptos::{html as leptos_html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure};

const MAX_SLASH_PALETTE_ITEMS: usize = 5;

#[derive(Clone, Copy)]
pub(crate) struct ComposerSlashSignals {
    pub visible: Signal<bool>,
    pub candidates: Signal<Vec<CompletionCandidate>>,
    pub selected_index: Signal<usize>,
    pub apply_selected: Signal<bool>,
}

#[derive(Clone, Copy)]
pub(crate) struct ComposerSlashCallbacks {
    pub select_next: Callback<()>,
    pub select_previous: Callback<()>,
    pub apply_selected: Callback<()>,
    pub apply_index: Callback<usize>,
    pub dismiss: Callback<()>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlashPaletteState {
    Empty,
    Ready(Vec<(usize, CompletionCandidate)>),
}

#[derive(Clone)]
struct SubmitDraftContext {
    form: NodeRef<leptos_html::Form>,
    textarea: NodeRef<leptos_html::Textarea>,
    disabled: Signal<bool>,
    on_submit: Callback<String>,
    restore_focus_after_submit: RwSignal<bool>,
}

#[component]
pub(crate) fn Composer(
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] status_text: Signal<String>,
    draft: RwSignal<String>,
    on_submit: Callback<String>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) -> impl IntoView {
    let form = NodeRef::<leptos_html::Form>::new();
    let textarea = NodeRef::<leptos_html::Textarea>::new();
    let restore_focus_after_submit = RwSignal::new(false);
    let submit_context = SubmitDraftContext {
        form,
        textarea,
        disabled,
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
            <ComposerEditor
                draft=draft
                slash_signals=slash_signals
                slash_callbacks=slash_callbacks
                submit_context=submit_context
            />
            <ComposerFooter
                status_text=status_text
                disabled=disabled
                show_cancel=show_cancel
                cancel_disabled=cancel_disabled
                on_cancel=on_cancel
            />
        </form>
    }
}

#[component]
fn ComposerEditor(
    draft: RwSignal<String>,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
    submit_context: SubmitDraftContext,
) -> impl IntoView {
    view! {
        <div class="composer__editor">
            <ComposerInput
                draft=draft
                slash_signals=slash_signals
                slash_callbacks=slash_callbacks
                submit_context=submit_context
            />
            <SlashPalette
                slash_signals=slash_signals
                on_apply_index=slash_callbacks.apply_index
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
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
    submit_context: SubmitDraftContext,
) -> impl IntoView {
    bind_submit_focus(submit_context.clone());
    let keydown_submit_context = submit_context.clone();
    let textarea = submit_context.textarea;
    let disabled = submit_context.disabled;

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
            aria-expanded=move || if slash_signals.visible.get() { "true" } else { "false" }
            aria-activedescendant=move || {
                if slash_signals.visible.get() && !slash_signals.candidates.get().is_empty() {
                    Some(slash_option_id(slash_signals.selected_index.get()))
                } else {
                    None
                }
            }
            node_ref=textarea
            placeholder="Write a prompt or type / for commands."
            prop:value=move || draft.get()
            on:input=move |ev| update_draft(draft, ev)
            on:keydown=move |ev| {
                handle_composer_keydown(
                    ev,
                    draft,
                    keydown_submit_context.clone(),
                    slash_signals,
                    slash_callbacks,
                );
            }
            prop:disabled=move || disabled.get()
        />
    }
}

fn update_draft(draft: RwSignal<String>, ev: web_sys::Event) {
    draft.set(event_target_value(&ev));
}

fn bind_submit_focus(submit_context: SubmitDraftContext) {
    bind_focus_restore_cancel(
        submit_context.form,
        submit_context.restore_focus_after_submit,
    );
    restore_submit_focus_when_ready(submit_context);
}

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

fn handle_composer_keydown(
    ev: web_sys::KeyboardEvent,
    draft: RwSignal<String>,
    submit_context: SubmitDraftContext,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) {
    if ev.is_composing() {
        return;
    }

    if handle_slash_palette_keydown(
        &ev,
        draft,
        submit_context.clone(),
        slash_signals,
        slash_callbacks,
    ) {
        return;
    }

    if ev.key() == "Enter" && !ev.shift_key() {
        ev.prevent_default();
        submit_draft(draft, submit_context);
    }
}

fn handle_slash_palette_keydown(
    ev: &web_sys::KeyboardEvent,
    draft: RwSignal<String>,
    submit_context: SubmitDraftContext,
    slash_signals: ComposerSlashSignals,
    slash_callbacks: ComposerSlashCallbacks,
) -> bool {
    if !slash_signals.visible.get_untracked() {
        return false;
    }

    match ev.key().as_str() {
        "ArrowDown" => slash_callbacks.select_next.run(()),
        "ArrowUp" => slash_callbacks.select_previous.run(()),
        "Tab" if !ev.shift_key() && slash_signals.apply_selected.get_untracked() => {
            slash_callbacks.apply_selected.run(());
        }
        "Enter" if !ev.shift_key() => {
            if slash_signals.apply_selected.get_untracked() {
                slash_callbacks.apply_selected.run(());
            } else {
                submit_draft(draft, submit_context);
            }
        }
        "Escape" => slash_callbacks.dismiss.run(()),
        _ => return false,
    }

    ev.prevent_default();
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
    slash_signals: ComposerSlashSignals,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    view! {
        <Show when=move || should_render_slash_palette(slash_signals)>
            <section class="composer__slash-palette" aria-label="Slash command suggestions">
                {move || {
                    render_slash_palette_state(
                        slash_palette_state(slash_signals),
                        slash_signals.selected_index.get(),
                        on_apply_index,
                    )
                }}
            </section>
        </Show>
    }
}

fn should_render_slash_palette(slash_signals: ComposerSlashSignals) -> bool {
    slash_signals.visible.get()
}

fn slash_palette_state(slash_signals: ComposerSlashSignals) -> SlashPaletteState {
    let items = slash_signals
        .candidates
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
    use acp_contracts::{CompletionCandidate, CompletionKind};
    use leptos::prelude::*;

    use super::{
        ComposerActions, ComposerFooter, ComposerSlashSignals, SlashPalette, SlashPaletteItem,
        SlashPaletteList, SlashPaletteState, current_submit_value, render_slash_palette_state,
        should_render_slash_palette, slash_option_id, slash_palette_state, submit_text,
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
    ) -> ComposerSlashSignals {
        ComposerSlashSignals {
            visible: Signal::derive(move || visible),
            candidates: Signal::derive(move || candidates.clone()),
            selected_index: Signal::derive(move || selected_index),
            apply_selected: Signal::derive(move || apply_selected),
        }
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
    fn should_render_slash_palette_follows_visible_signal() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = make_slash_signals(true, vec![], 0, false);
            assert!(should_render_slash_palette(signals));

            let signals_hidden = make_slash_signals(false, vec![], 0, false);
            assert!(!should_render_slash_palette(signals_hidden));
        });
    }

    #[test]
    fn slash_palette_state_empty_when_no_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let signals = make_slash_signals(true, vec![], 0, false);
            assert_eq!(slash_palette_state(signals), SlashPaletteState::Empty);
        });
    }

    #[test]
    fn slash_palette_state_ready_with_indexed_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let candidates = vec![make_candidate("/help"), make_candidate("/clear")];
            let signals = make_slash_signals(true, candidates, 0, false);
            let state = slash_palette_state(signals);
            match state {
                SlashPaletteState::Ready(items) => {
                    assert_eq!(items.len(), 2);
                    assert_eq!(items[0].0, 0);
                    assert_eq!(items[1].0, 1);
                }
                SlashPaletteState::Empty => panic!("expected Ready"),
            }
        });
    }

    #[test]
    fn slash_palette_state_caps_at_max_items() {
        let owner = Owner::new();
        owner.with(|| {
            let candidates: Vec<_> = (0..10)
                .map(|i| make_candidate(&format!("/cmd{i}")))
                .collect();
            let signals = make_slash_signals(true, candidates, 0, false);
            match slash_palette_state(signals) {
                SlashPaletteState::Ready(items) => assert_eq!(items.len(), 5),
                SlashPaletteState::Empty => panic!("expected Ready"),
            }
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

            let _ = view! {
                <SlashPalette
                    slash_signals=make_slash_signals(true, candidates.clone(), 1, true)
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
    fn composer_footer_and_actions_build_without_panicking() {
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
            let _ = view! {
                <ComposerActions
                    disabled=Signal::derive(|| false)
                    show_cancel=Signal::derive(|| true)
                    cancel_disabled=Signal::derive(|| false)
                    on_cancel=Callback::new(|()| {})
                />
            };
        });
    }
}
