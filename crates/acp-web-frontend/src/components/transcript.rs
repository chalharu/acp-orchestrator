//! Conversation transcript component.

use leptos::{html as leptos_html, prelude::*};

#[cfg(target_family = "wasm")]
use crate::transcript_view::tail_scroll_top;
use crate::transcript_view::{compute_virtual_window, render_markdown};
#[cfg(target_family = "wasm")]
use wasm_bindgen::{JsCast, closure::Closure};

const DEFAULT_VIEWPORT_HEIGHT: f64 = 640.0;
const BOTTOM_TOLERANCE_PX: f64 = 24.0;

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TranscriptEntry {
    pub(crate) id: String,
    pub(crate) role: EntryRole,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryRole {
    User,
    Assistant,
    Status,
}

impl EntryRole {
    pub(crate) fn css_class(&self) -> &'static str {
        match self {
            Self::User => "transcript-entry--user",
            Self::Assistant => "transcript-entry--assistant",
            Self::Status => "transcript-entry--status",
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Status => "status",
        }
    }
}

#[component]
pub(crate) fn Transcript(#[prop(into)] entries: Signal<Vec<TranscriptEntry>>) -> impl IntoView {
    let viewport = NodeRef::<leptos_html::Section>::new();
    let scroll_top = RwSignal::new(0.0);
    let viewport_height = RwSignal::new(DEFAULT_VIEWPORT_HEIGHT);
    let follow_tail = RwSignal::new(true);
    let virtual_window = transcript_virtual_window_memo(entries, scroll_top, viewport_height);
    let visible_entries = Signal::derive(move || virtual_window.get().visible);
    let top_spacer_height = Signal::derive(move || virtual_window.get().top_spacer_height);
    let bottom_spacer_height = Signal::derive(move || virtual_window.get().bottom_spacer_height);

    bind_viewport_effects(viewport, entries, viewport_height, scroll_top, follow_tail);

    let on_scroll = move |ev| {
        let element = event_target::<web_sys::HtmlElement>(&ev);
        update_scroll_metrics(&element, scroll_top, viewport_height, follow_tail);
    };

    view! {
        <section
            class="transcript-viewport"
            aria-label="conversation transcript"
            node_ref=viewport
            on:scroll=on_scroll
        >
            {transcript_content(
                entries,
                visible_entries,
                top_spacer_height,
                bottom_spacer_height,
            )}
        </section>
    }
}

fn transcript_virtual_window(
    entries: &[TranscriptEntry],
    scroll_top: f64,
    viewport_height: f64,
) -> crate::transcript_view::VirtualWindow<TranscriptEntry> {
    compute_virtual_window(entries, scroll_top, viewport_height, estimated_entry_height)
}

fn transcript_virtual_window_memo(
    entries: Signal<Vec<TranscriptEntry>>,
    scroll_top: RwSignal<f64>,
    viewport_height: RwSignal<f64>,
) -> Memo<crate::transcript_view::VirtualWindow<TranscriptEntry>> {
    Memo::new(move |_| {
        transcript_virtual_window(&entries.get(), scroll_top.get(), viewport_height.get())
    })
}

fn transcript_empty_view() -> impl IntoView {
    view! {
        <div class="transcript-empty">
            <p class="muted">"No messages yet."</p>
        </div>
    }
}

fn transcript_content(
    entries: Signal<Vec<TranscriptEntry>>,
    visible_entries: Signal<Vec<TranscriptEntry>>,
    top_spacer_height: Signal<f64>,
    bottom_spacer_height: Signal<f64>,
) -> impl IntoView {
    #[rustfmt::skip]
    view! {
        <Show
            when=move || !entries.get().is_empty()
            fallback=transcript_empty_view
        >
            <ol class="transcript"><TranscriptSpacer height=top_spacer_height /><For each=move || visible_entries.get() key=|entry| entry.id.clone() children=render_transcript_entry_item /><TranscriptSpacer height=bottom_spacer_height /></ol>
        </Show>
    }
}

fn render_transcript_entry_item(entry: TranscriptEntry) -> AnyView {
    let TranscriptEntry { role, text, .. } = entry;
    let css = role.css_class().to_string();
    let label = format!("{}: ", role.label());

    if matches!(role, EntryRole::Status) {
        view! {
            <li class=format!("transcript-entry {css}")>
                <p class="transcript-entry__body transcript-entry__body--plain">
                    <span class="sr-only">{label}</span>
                    {text}
                </p>
            </li>
        }
        .into_any()
    } else {
        view! {
            <li class=format!("transcript-entry {css}")>
                <span class="sr-only">{label}</span>
                <TranscriptMarkdown markdown=text />
            </li>
        }
        .into_any()
    }
}

#[component]
fn TranscriptMarkdown(markdown: String) -> impl IntoView {
    let container = NodeRef::<leptos_html::Div>::new();
    let rendered_html = Memo::new(move |_| render_markdown(&markdown));

    #[cfg(target_family = "wasm")]
    Effect::new(move |_| {
        if let Some(element) = container.get() {
            element.set_inner_html(&rendered_html.get());
        }
    });

    #[cfg(not(target_family = "wasm"))]
    let _ = &rendered_html;

    view! {
        <div
            class="transcript-entry__body transcript-entry__body--markdown"
            node_ref=container
        ></div>
    }
}

#[component]
fn TranscriptSpacer(#[prop(into)] height: Signal<f64>) -> impl IntoView {
    view! {
        <li
            class="transcript-spacer"
            aria-hidden="true"
            style:height=move || format!("{}px", height.get())
        ></li>
    }
}

fn bind_viewport_effects(
    viewport: NodeRef<leptos_html::Section>,
    entries: Signal<Vec<TranscriptEntry>>,
    viewport_height: RwSignal<f64>,
    scroll_top: RwSignal<f64>,
    follow_tail: RwSignal<bool>,
) {
    #[cfg(not(target_family = "wasm"))]
    {
        let _ = (
            &viewport,
            &entries,
            &viewport_height,
            &scroll_top,
            &follow_tail,
        );
    }

    #[cfg(target_family = "wasm")]
    {
        let viewport_for_metrics = viewport;
        Effect::new(move |_| {
            if let Some(element) = viewport_for_metrics.get() {
                viewport_height.set(measured_viewport_height(&element));
            }
        });

        Effect::new(move |_| {
            if !follow_tail.get() || entries.get().is_empty() {
                return;
            }
            if let Some(element) = viewport.get() {
                viewport_height.set(measured_viewport_height(&element));
                schedule_tail_scroll(element, scroll_top);
            }
        });
    }
}

#[cfg(target_family = "wasm")]
fn measured_viewport_height(element: &web_sys::HtmlElement) -> f64 {
    f64::from(element.client_height()).max(DEFAULT_VIEWPORT_HEIGHT / 4.0)
}

#[cfg(target_family = "wasm")]
fn schedule_tail_scroll(element: web_sys::HtmlElement, scroll_top: RwSignal<f64>) {
    let Some(window) = web_sys::window() else {
        scroll_element_to_tail(&element, scroll_top);
        return;
    };

    let scheduled_element = element.clone();
    let callback = Closure::once(move || {
        scroll_element_to_tail(&scheduled_element, scroll_top);
    });

    if window
        .request_animation_frame(callback.as_ref().unchecked_ref())
        .is_ok()
    {
        callback.forget();
    } else {
        scroll_element_to_tail(&element, scroll_top);
    }
}

#[cfg(target_family = "wasm")]
fn scroll_element_to_tail(element: &web_sys::HtmlElement, scroll_top: RwSignal<f64>) {
    element.set_scroll_top(tail_scroll_top(
        element.scroll_height(),
        element.client_height(),
    ));
    scroll_top.set(f64::from(element.scroll_top()));
}

fn update_scroll_metrics(
    element: &web_sys::HtmlElement,
    scroll_top: RwSignal<f64>,
    viewport_height: RwSignal<f64>,
    follow_tail: RwSignal<bool>,
) {
    let current_scroll_top = f64::from(element.scroll_top());
    let current_viewport_height = f64::from(element.client_height());
    let remaining_distance =
        f64::from(element.scroll_height()) - current_scroll_top - current_viewport_height;

    scroll_top.set(current_scroll_top.max(0.0));
    viewport_height.set(current_viewport_height.max(DEFAULT_VIEWPORT_HEIGHT / 4.0));
    follow_tail.set(remaining_distance <= BOTTOM_TOLERANCE_PX);
}

fn estimated_entry_height(entry: &TranscriptEntry) -> f64 {
    let (base_height, chars_per_line) = match entry.role {
        EntryRole::User | EntryRole::Assistant => (48.0, 52),
        EntryRole::Status => (36.0, 62),
    };
    let estimated_lines = entry
        .text
        .lines()
        .map(|line| estimate_visual_lines(line, chars_per_line))
        .sum::<usize>()
        .max(1);

    base_height + (estimated_lines as f64 * 22.0)
}

fn estimate_visual_lines(line: &str, chars_per_line: usize) -> usize {
    let char_count = line.chars().count().max(1);
    (char_count + chars_per_line.saturating_sub(1)) / chars_per_line.max(1)
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn entry_role_helpers_return_expected_labels_and_classes() {
        assert_eq!(EntryRole::User.css_class(), "transcript-entry--user");
        assert_eq!(
            EntryRole::Assistant.css_class(),
            "transcript-entry--assistant"
        );
        assert_eq!(EntryRole::Status.css_class(), "transcript-entry--status");
        assert_eq!(EntryRole::User.label(), "user");
        assert_eq!(EntryRole::Assistant.label(), "assistant");
        assert_eq!(EntryRole::Status.label(), "status");
    }

    fn entry(id: &str, role: EntryRole, text: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: id.to_string(),
            role,
            text: text.to_string(),
        }
    }

    #[test]
    fn transcript_builds_for_empty_and_populated_entry_lists() {
        let owner = Owner::new();
        owner.with(|| {
            let entries = RwSignal::new(Vec::<TranscriptEntry>::new());
            let signal = Signal::derive(move || entries.get());

            let _ = view! { <Transcript entries=signal /> };
            entries.set(vec![
                entry("assistant", EntryRole::Assistant, "hello"),
                entry("status", EntryRole::Status, "done"),
            ]);
            let _ = view! { <Transcript entries=signal /> };
        });
    }

    #[test]
    fn transcript_entry_stores_the_underlying_values() {
        let transcript_entry = entry("assistant", EntryRole::Assistant, "hello");

        assert_eq!(transcript_entry.id, "assistant");
        assert_eq!(transcript_entry.role, EntryRole::Assistant);
        assert_eq!(transcript_entry.text, "hello");
    }

    #[test]
    fn transcript_virtual_window_helper_and_empty_view_are_host_safe() {
        let owner = Owner::new();
        owner.with(|| {
            let entries = vec![entry("assistant", EntryRole::Assistant, "hello")];
            let window = transcript_virtual_window(&entries, 0.0, DEFAULT_VIEWPORT_HEIGHT);

            assert_eq!(window.visible.len(), 1);
            let _ = transcript_empty_view();
        });
    }

    #[test]
    fn transcript_virtual_window_memo_evaluates_for_host_signals() {
        let owner = Owner::new();
        owner.with(|| {
            let entries = RwSignal::new(vec![entry("assistant", EntryRole::Assistant, "hello")]);
            let memo = transcript_virtual_window_memo(
                Signal::derive(move || entries.get()),
                RwSignal::new(0.0),
                RwSignal::new(DEFAULT_VIEWPORT_HEIGHT),
            );

            assert_eq!(memo.get().visible.len(), 1);
        });
    }

    #[test]
    fn transcript_entry_item_builds_status_and_markdown_variants() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = render_transcript_entry_item(entry("status", EntryRole::Status, "done"));
            let _ = view! {
                {render_transcript_entry_item(entry("assistant", EntryRole::Assistant, "**bold**"))}
            };
        });
    }

    #[test]
    fn transcript_markdown_and_spacer_build_without_dom_nodes() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! { <TranscriptMarkdown markdown="**bold**".to_string() /> };
            let _ = view! { <TranscriptSpacer height=Signal::derive(|| 24.0) /> };
        });
    }
    #[test]
    fn estimated_entry_height_uses_status_layout() {
        assert_eq!(
            estimated_entry_height(&entry("status", EntryRole::Status, "")),
            58.0
        );
    }

    #[test]
    fn estimated_entry_height_uses_chat_layout_and_wraps_long_lines() {
        let wrapped =
            estimated_entry_height(&entry("assistant", EntryRole::Assistant, &"x".repeat(53)));

        assert_eq!(wrapped, 92.0);
    }

    #[test]
    fn estimate_visual_lines_handles_empty_lines_and_zero_width_inputs() {
        assert_eq!(estimate_visual_lines("", 52), 1);
        assert_eq!(estimate_visual_lines("abcd", 0), 4);
        assert_eq!(estimate_visual_lines("abcdef", 4), 2);
    }
}
