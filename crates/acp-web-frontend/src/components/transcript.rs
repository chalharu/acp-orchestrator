//! Conversation transcript component.

use leptos::{html, prelude::*};

use crate::{EntryRole, TranscriptEntry};

const DEFAULT_VIEWPORT_HEIGHT: f64 = 640.0;
const OVERSCAN_PX: f64 = 320.0;
const BOTTOM_TOLERANCE_PX: f64 = 24.0;

#[derive(Clone, Debug, PartialEq)]
struct VirtualWindow {
    visible: Vec<TranscriptEntry>,
    top_spacer_height: f64,
    bottom_spacer_height: f64,
}

#[component]
pub fn Transcript(#[prop(into)] entries: Signal<Vec<TranscriptEntry>>) -> impl IntoView {
    let viewport = NodeRef::<html::Section>::new();
    let scroll_top = RwSignal::new(0.0);
    let viewport_height = RwSignal::new(DEFAULT_VIEWPORT_HEIGHT);
    let follow_tail = RwSignal::new(true);
    let virtual_window = Memo::new(move |_| {
        compute_virtual_window(&entries.get(), scroll_top.get(), viewport_height.get())
    });
    let visible_entries = Signal::derive(move || virtual_window.get().visible);
    let top_spacer_height = Signal::derive(move || virtual_window.get().top_spacer_height);
    let bottom_spacer_height = Signal::derive(move || virtual_window.get().bottom_spacer_height);

    let on_scroll = move |ev| {
        let element = event_target::<web_sys::HtmlElement>(&ev);
        update_scroll_metrics(&element, scroll_top, viewport_height, follow_tail);
    };

    Effect::new(move |_| {
        let entries = entries.get();
        if let Some(element) = viewport.get() {
            viewport_height.set(f64::from(element.client_height()));
            if follow_tail.get() && !entries.is_empty() {
                element.set_scroll_top(element.scroll_height());
                scroll_top.set(f64::from(element.scroll_top()));
            }
        }
    });

    view! {
        <section
            class="transcript-viewport"
            aria-label="conversation transcript"
            node_ref=viewport
            on:scroll=on_scroll
        >
            <Show
                when=move || !entries.get().is_empty()
                fallback=move || {
                    view! {
                        <div class="transcript-empty">
                            <p class="muted">"No messages yet."</p>
                        </div>
                    }
                }
            >
                <ol class="transcript">
                    <TranscriptSpacer height=top_spacer_height />
                    <For
                        each=move || visible_entries.get()
                        key=|entry| entry.id.clone()
                        children=move |entry| {
                            let css = entry.role.css_class();
                            let label = entry.role.label();
                            let text = entry.text;

                            view! {
                                <li class=format!("transcript-entry {css}")>
                                    <p class="transcript-entry__body">
                                        <span class="sr-only">{format!("{label}: ")}</span>
                                        {text}
                                    </p>
                                </li>
                            }
                        }
                    />
                    <TranscriptSpacer height=bottom_spacer_height />
                </ol>
            </Show>
        </section>
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

fn compute_virtual_window(
    entries: &[TranscriptEntry],
    scroll_top: f64,
    viewport_height: f64,
) -> VirtualWindow {
    if entries.is_empty() {
        return VirtualWindow {
            visible: Vec::new(),
            top_spacer_height: 0.0,
            bottom_spacer_height: 0.0,
        };
    }

    let start_threshold = (scroll_top - OVERSCAN_PX).max(0.0);
    let end_threshold = scroll_top + viewport_height + OVERSCAN_PX;
    let mut offset = 0.0;
    let mut start_index = 0;

    while start_index < entries.len() {
        let next_offset = offset + estimated_entry_height(&entries[start_index]);
        if next_offset >= start_threshold {
            break;
        }
        offset = next_offset;
        start_index += 1;
    }

    if start_index == entries.len() {
        start_index = entries.len().saturating_sub(1);
        offset -= estimated_entry_height(&entries[start_index]);
    }

    let top_spacer_height = offset;
    let mut end_index = start_index;
    let mut rendered_height = offset;

    while end_index < entries.len() {
        rendered_height += estimated_entry_height(&entries[end_index]);
        end_index += 1;
        if rendered_height >= end_threshold {
            break;
        }
    }

    let total_height = entries.iter().map(estimated_entry_height).sum::<f64>();

    VirtualWindow {
        visible: entries[start_index..end_index].to_vec(),
        top_spacer_height,
        bottom_spacer_height: (total_height - rendered_height).max(0.0),
    }
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
    use super::compute_virtual_window;
    use crate::{EntryRole, TranscriptEntry};

    #[test]
    fn virtual_window_returns_all_entries_for_short_transcripts() {
        let entries = vec![entry("1", EntryRole::Assistant, "hello")];
        let window = compute_virtual_window(&entries, 0.0, 800.0);

        assert_eq!(window.visible, entries);
        assert_eq!(window.top_spacer_height, 0.0);
        assert_eq!(window.bottom_spacer_height, 0.0);
    }

    #[test]
    fn virtual_window_skips_entries_before_the_visible_region() {
        let entries = (0..20)
            .map(|index| {
                let id = index.to_string();
                let text = "line ".repeat(12);
                entry(&id, EntryRole::Assistant, &text)
            })
            .collect::<Vec<_>>();
        let window = compute_virtual_window(&entries, 900.0, 280.0);

        assert!(window.visible.len() < entries.len());
        assert!(window.top_spacer_height > 0.0);
        assert!(window.bottom_spacer_height > 0.0);
    }

    fn entry(id: &str, role: EntryRole, text: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: id.to_string(),
            role,
            text: text.to_string(),
        }
    }
}
