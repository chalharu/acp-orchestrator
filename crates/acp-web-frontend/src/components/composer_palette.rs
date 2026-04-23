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
    let render_item = slash_palette_list_children(selected_index, on_apply_index);

    view! {
        <ul id=SLASH_PALETTE_LISTBOX_ID role="listbox" class="composer__slash-list">
            <For
                each=move || items.clone()
                key=|(index, candidate)| (index.to_owned(), candidate.label.clone())
                children=render_item
            />
        </ul>
    }
}

fn slash_palette_list_children(
    selected_index: usize,
    on_apply_index: Callback<usize>,
) -> impl Fn((usize, CompletionCandidate)) -> AnyView + Copy + 'static {
    move |(index, candidate)| {
        slash_palette_item_view(index, candidate, index == selected_index, on_apply_index)
            .into_any()
    }
}

fn slash_palette_item_view(
    index: usize,
    candidate: CompletionCandidate,
    is_selected: bool,
    on_apply_index: Callback<usize>,
) -> impl IntoView {
    view! {
        <SlashPaletteItem
            index=index
            candidate=candidate
            is_selected=is_selected
            on_apply_index=on_apply_index
        />
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
    let item_class = slash_item_class(is_selected);
    let on_mousedown = palette_mousedown_handler(on_apply_index, index);
    let on_keydown = palette_keydown_handler(on_apply_index, index);

    view! {
        <li
            id=option_id
            role="option"
            aria-selected=if is_selected { "true" } else { "false" }
        >
            <button
                type="button"
                class=item_class
                tabindex="-1"
                on:mousedown=on_mousedown
                on:keydown=on_keydown
            >
                <span class="composer__slash-label">{label}</span>
                <span class="composer__slash-detail">{detail}</span>
            </button>
        </li>
    }
}

fn slash_item_class(is_selected: bool) -> &'static str {
    if is_selected {
        "composer__slash-item composer__slash-item--selected"
    } else {
        "composer__slash-item"
    }
}

fn palette_apply_handler<E>(
    on_apply_index: Callback<usize>,
    index: usize,
) -> impl Fn(E) + Copy + 'static
where
    E: 'static,
{
    move |_event: E| on_apply_index.run(index)
}

#[cfg(target_family = "wasm")]
fn palette_mousedown_handler(
    on_apply_index: Callback<usize>,
    index: usize,
) -> impl Fn(web_sys::MouseEvent) + Copy + 'static {
    move |event: web_sys::MouseEvent| {
        event.prevent_default();
        on_apply_index.run(index);
    }
}

#[cfg(not(target_family = "wasm"))]
fn palette_mousedown_handler(
    on_apply_index: Callback<usize>,
    index: usize,
) -> impl Fn(web_sys::MouseEvent) + Copy + 'static {
    palette_apply_handler(on_apply_index, index)
}

fn should_apply_palette_key(key: &str) -> bool {
    matches!(key, "Enter" | " ")
}

fn palette_key_handler<E>(
    on_apply_index: Callback<usize>,
    index: usize,
    key: fn(&E) -> String,
) -> impl Fn(E) + Copy + 'static
where
    E: 'static,
{
    move |event: E| {
        if should_apply_palette_key(key(&event).as_str()) {
            on_apply_index.run(index);
        }
    }
}

fn empty_palette_key_text<E>(_event: &E) -> String {
    String::new()
}

#[cfg(target_family = "wasm")]
fn palette_keydown_handler(
    on_apply_index: Callback<usize>,
    index: usize,
) -> impl Fn(web_sys::KeyboardEvent) + Copy + 'static {
    move |event: web_sys::KeyboardEvent| {
        if should_apply_palette_key(event.key().as_str()) {
            event.prevent_default();
            on_apply_index.run(index);
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn palette_keydown_handler(
    on_apply_index: Callback<usize>,
    index: usize,
) -> impl Fn(web_sys::KeyboardEvent) + Copy + 'static {
    palette_key_handler(on_apply_index, index, empty_palette_key_text)
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
            let _ = slash_palette_item_view(
                0,
                make_candidate("/help"),
                true,
                Callback::new(|_: usize| {}),
            );
            let render_item = slash_palette_list_children(0, Callback::new(|_: usize| {}));
            let _ = render_item((0, make_candidate("/help")));
        });
    }

    #[test]
    fn slash_item_class_matches_selection_state() {
        assert_eq!(
            slash_item_class(true),
            "composer__slash-item composer__slash-item--selected"
        );
        assert_eq!(slash_item_class(false), "composer__slash-item");
    }

    struct FakeKeyboardEvent {
        key: String,
    }

    #[test]
    fn palette_apply_handler_runs_callback() {
        let owner = Owner::new();
        owner.with(|| {
            let applied = RwSignal::new(Vec::<usize>::new());

            palette_apply_handler(
                Callback::new(move |index| applied.update(|items| items.push(index))),
                3,
            )(());

            assert_eq!(applied.get(), vec![3]);
        });
    }

    #[test]
    fn empty_palette_key_text_returns_empty_string() {
        assert!(empty_palette_key_text(&FakeKeyboardEvent {
            key: "Enter".to_string(),
        })
        .is_empty());
    }

    #[test]
    fn palette_key_handler_only_applies_enter_and_space() {
        let owner = Owner::new();
        owner.with(|| {
            let applied = RwSignal::new(Vec::<usize>::new());
            let enter = FakeKeyboardEvent {
                key: "Enter".to_string(),
            };
            let other = FakeKeyboardEvent {
                key: "Escape".to_string(),
            };
            let callback = Callback::new(move |index| applied.update(|items| items.push(index)));

            palette_key_handler(callback, 1, |event: &FakeKeyboardEvent| event.key.clone())(enter);
            palette_key_handler(callback, 2, |event: &FakeKeyboardEvent| event.key.clone())(other);

            assert_eq!(applied.get(), vec![1]);
            assert!(should_apply_palette_key(" "));
            assert!(!should_apply_palette_key("Escape"));
        });
    }
}
