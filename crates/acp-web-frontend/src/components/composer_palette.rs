//! Slash command palette component for composer.

use acp_contracts_slash::CompletionCandidate;
use leptos::prelude::*;

const MAX_SLASH_PALETTE_ITEMS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlashPaletteState {
    Empty,
    Ready(Vec<(usize, CompletionCandidate)>),
}

pub(super) const SLASH_PALETTE_LISTBOX_ID: &str = "slash-palette-listbox";

pub(super) fn slash_option_id(index: usize) -> String {
    format!("slash-option-{index}")
}

#[component]
pub(super) fn SlashPalette(
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

#[cfg(test)]
mod tests {
    use super::*;
    use acp_contracts_slash::CompletionKind;

    fn make_candidate(label: &str) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            insert_text: label.to_string(),
            detail: "detail".to_string(),
            kind: CompletionKind::Command,
        }
    }

    #[test]
    fn slash_palette_state_empty_when_no_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let candidates = Signal::derive(Vec::new);
            assert_eq!(slash_palette_state(candidates), SlashPaletteState::Empty);
        });
    }

    #[test]
    fn slash_palette_state_ready_with_indexed_candidates() {
        let owner = Owner::new();
        owner.with(|| {
            let candidates_vec = vec![make_candidate("/help"), make_candidate("/clear")];
            let candidates = Signal::derive(move || candidates_vec.clone());
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
            let candidates_vec: Vec<_> = (0..10)
                .map(|i| make_candidate(&format!("/cmd{i}")))
                .collect();
            let candidates = Signal::derive(move || candidates_vec.clone());
            let state = slash_palette_state(candidates);
            assert!(matches!(state, SlashPaletteState::Ready(items) if items.len() == 5));
        });
    }

    #[test]
    fn should_render_slash_palette_follows_visible_signal() {
        let owner = Owner::new();
        owner.with(|| {
            assert!(!should_render_slash_palette(Signal::derive(|| false)));
            assert!(should_render_slash_palette(Signal::derive(|| true)));
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
            let _ = view! {
                <SlashPalette
                    slash_visible=Signal::derive(|| true)
                    slash_candidates=Signal::derive(|| vec![make_candidate("/help"), make_candidate("/clear")])
                    slash_selected_index=Signal::derive(|| 0usize)
                    on_apply_index=Callback::new(|_: usize| {})
                />
            };
            let _ = view! {
                <SlashPaletteList
                    items=vec![(0, make_candidate("/help")), (1, make_candidate("/clear"))]
                    selected_index=0
                    on_apply_index=Callback::new(|_: usize| {})
                />
            };
            let _ = view! {
                <SlashPaletteItem
                    index=0
                    candidate=make_candidate("/help")
                    is_selected=true
                    on_apply_index=Callback::new(|_: usize| {})
                />
            };
        });
    }
}
