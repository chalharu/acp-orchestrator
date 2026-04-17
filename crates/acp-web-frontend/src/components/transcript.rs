//! Conversation transcript component.

use leptos::{html as leptos_html, prelude::*};
use pulldown_cmark::{
    Event as MarkdownEvent, LinkType, Options as MarkdownOptions, Parser, Tag, TagEnd,
    html::push_html,
};
use wasm_bindgen::{JsCast, closure::Closure};

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
    let viewport = NodeRef::<leptos_html::Section>::new();
    let scroll_top = RwSignal::new(0.0);
    let viewport_height = RwSignal::new(DEFAULT_VIEWPORT_HEIGHT);
    let follow_tail = RwSignal::new(true);
    let virtual_window = Memo::new(move |_| {
        compute_virtual_window(&entries.get(), scroll_top.get(), viewport_height.get())
    });
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
                        children=move |entry| view! { <TranscriptEntryItem entry=entry /> }
                    />
                    <TranscriptSpacer height=bottom_spacer_height />
                </ol>
            </Show>
        </section>
    }
}

#[component]
fn TranscriptEntryItem(entry: TranscriptEntry) -> impl IntoView {
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

    Effect::new(move |_| {
        if let Some(element) = container.get() {
            element.set_inner_html(&rendered_html.get());
        }
    });

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

fn measured_viewport_height(element: &web_sys::HtmlElement) -> f64 {
    f64::from(element.client_height()).max(DEFAULT_VIEWPORT_HEIGHT / 4.0)
}

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

fn scroll_element_to_tail(element: &web_sys::HtmlElement, scroll_top: RwSignal<f64>) {
    element.set_scroll_top(tail_scroll_top(
        element.scroll_height(),
        element.client_height(),
    ));
    scroll_top.set(f64::from(element.scroll_top()));
}

fn tail_scroll_top(scroll_height: i32, client_height: i32) -> i32 {
    (scroll_height - client_height).max(0)
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

fn render_markdown(source: &str) -> String {
    let parser = Parser::new_ext(source, markdown_options());
    let mut rendered = String::new();
    push_html(&mut rendered, sanitize_markdown_events(parser).into_iter());
    rendered
}

fn markdown_options() -> MarkdownOptions {
    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    options.insert(MarkdownOptions::ENABLE_TABLES);
    options.insert(MarkdownOptions::ENABLE_TASKLISTS);
    options.insert(MarkdownOptions::ENABLE_SMART_PUNCTUATION);
    options
}

fn sanitize_markdown_events<'a>(parser: Parser<'a>) -> Vec<MarkdownEvent<'a>> {
    let mut ignored_ends = Vec::new();

    parser
        .filter_map(|event| match event {
            MarkdownEvent::Start(tag) => sanitize_markdown_start(tag, &mut ignored_ends),
            MarkdownEvent::End(tag_end) => sanitize_markdown_end(tag_end, &mut ignored_ends),
            MarkdownEvent::Text(text) => Some(MarkdownEvent::Text(text)),
            MarkdownEvent::Code(code) => Some(MarkdownEvent::Code(code)),
            MarkdownEvent::InlineMath(text)
            | MarkdownEvent::DisplayMath(text)
            | MarkdownEvent::FootnoteReference(text) => Some(MarkdownEvent::Text(text)),
            MarkdownEvent::Html(html) | MarkdownEvent::InlineHtml(html) => {
                Some(MarkdownEvent::Text(html))
            }
            MarkdownEvent::SoftBreak => Some(MarkdownEvent::HardBreak),
            MarkdownEvent::HardBreak => Some(MarkdownEvent::HardBreak),
            MarkdownEvent::Rule => Some(MarkdownEvent::Rule),
            MarkdownEvent::TaskListMarker(checked) => Some(MarkdownEvent::TaskListMarker(checked)),
        })
        .collect()
}

fn sanitize_markdown_start<'a>(
    tag: Tag<'a>,
    ignored_ends: &mut Vec<TagEnd>,
) -> Option<MarkdownEvent<'a>> {
    match tag {
        Tag::Paragraph => Some(MarkdownEvent::Start(Tag::Paragraph)),
        Tag::Heading { level, .. } => Some(MarkdownEvent::Start(Tag::Heading {
            level,
            id: None,
            classes: Vec::new(),
            attrs: Vec::new(),
        })),
        Tag::BlockQuote(kind) => Some(MarkdownEvent::Start(Tag::BlockQuote(kind))),
        Tag::CodeBlock(kind) => Some(MarkdownEvent::Start(Tag::CodeBlock(kind))),
        Tag::List(start) => Some(MarkdownEvent::Start(Tag::List(start))),
        Tag::Item => Some(MarkdownEvent::Start(Tag::Item)),
        Tag::DefinitionList => Some(MarkdownEvent::Start(Tag::DefinitionList)),
        Tag::DefinitionListTitle => Some(MarkdownEvent::Start(Tag::DefinitionListTitle)),
        Tag::DefinitionListDefinition => Some(MarkdownEvent::Start(Tag::DefinitionListDefinition)),
        Tag::Table(alignments) => Some(MarkdownEvent::Start(Tag::Table(alignments))),
        Tag::TableHead => Some(MarkdownEvent::Start(Tag::TableHead)),
        Tag::TableRow => Some(MarkdownEvent::Start(Tag::TableRow)),
        Tag::TableCell => Some(MarkdownEvent::Start(Tag::TableCell)),
        Tag::Emphasis => Some(MarkdownEvent::Start(Tag::Emphasis)),
        Tag::Strong => Some(MarkdownEvent::Start(Tag::Strong)),
        Tag::Strikethrough => Some(MarkdownEvent::Start(Tag::Strikethrough)),
        Tag::Superscript => Some(MarkdownEvent::Start(Tag::Superscript)),
        Tag::Subscript => Some(MarkdownEvent::Start(Tag::Subscript)),
        Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        } if is_safe_markdown_link(link_type, dest_url.as_ref()) => {
            Some(MarkdownEvent::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }))
        }
        Tag::Link { .. } => {
            ignored_ends.push(TagEnd::Link);
            None
        }
        Tag::Image { .. } => {
            ignored_ends.push(TagEnd::Image);
            None
        }
        Tag::HtmlBlock => {
            ignored_ends.push(TagEnd::HtmlBlock);
            None
        }
        Tag::FootnoteDefinition(_) => {
            ignored_ends.push(TagEnd::FootnoteDefinition);
            None
        }
        Tag::MetadataBlock(kind) => {
            ignored_ends.push(TagEnd::MetadataBlock(kind));
            None
        }
    }
}

fn sanitize_markdown_end<'a>(
    tag_end: TagEnd,
    ignored_ends: &mut Vec<TagEnd>,
) -> Option<MarkdownEvent<'a>> {
    if ignored_ends.last() == Some(&tag_end) {
        ignored_ends.pop();
        return None;
    }

    match tag_end {
        TagEnd::Paragraph
        | TagEnd::Heading(_)
        | TagEnd::BlockQuote(_)
        | TagEnd::CodeBlock
        | TagEnd::List(_)
        | TagEnd::Item
        | TagEnd::DefinitionList
        | TagEnd::DefinitionListTitle
        | TagEnd::DefinitionListDefinition
        | TagEnd::Table
        | TagEnd::TableHead
        | TagEnd::TableRow
        | TagEnd::TableCell
        | TagEnd::Emphasis
        | TagEnd::Strong
        | TagEnd::Strikethrough
        | TagEnd::Superscript
        | TagEnd::Subscript
        | TagEnd::Link => Some(MarkdownEvent::End(tag_end)),
        TagEnd::HtmlBlock
        | TagEnd::Image
        | TagEnd::FootnoteDefinition
        | TagEnd::MetadataBlock(_) => None,
    }
}

fn is_safe_markdown_link(link_type: LinkType, url: &str) -> bool {
    !matches!(link_type, LinkType::Email | LinkType::WikiLink { .. }) && is_safe_markdown_url(url)
}

fn is_safe_markdown_url(url: &str) -> bool {
    let trimmed = url.trim();
    let scheme_candidate = trimmed
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(trimmed)
        .to_ascii_lowercase();

    if scheme_candidate.contains("%3a") {
        return false;
    }

    if trimmed.starts_with('#')
        || trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
    {
        return true;
    }

    match trimmed.split_once(':') {
        Some((scheme, _)) => matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https"),
        None => true,
    }
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
    use super::{compute_virtual_window, render_markdown, tail_scroll_top};
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

    #[test]
    fn tail_scroll_top_clamps_at_zero() {
        assert_eq!(tail_scroll_top(120, 400), 0);
        assert_eq!(tail_scroll_top(640, 320), 320);
    }

    #[test]
    fn render_markdown_formats_common_markdown() {
        let rendered = render_markdown("**Bold**\n\n- item\n\n`code`");

        assert!(rendered.contains("<strong>Bold</strong>"));
        assert!(rendered.contains("<ul>"));
        assert!(rendered.contains("<code>code</code>"));
    }

    #[test]
    fn render_markdown_escapes_html_and_unsafe_links() {
        let rendered =
            render_markdown("<b>hi</b> [x](javascript:alert(1)) ![alt](https://example.com/x.png)");

        assert!(rendered.contains("&lt;b&gt;hi&lt;/b&gt;"));
        assert!(!rendered.contains("javascript:alert"));
        assert!(!rendered.contains("<img"));
        assert!(rendered.contains("alt"));
    }

    #[test]
    fn render_markdown_rejects_mailto_and_encoded_scheme_links() {
        let rendered = render_markdown(
            "[mail](mailto:test@example.com) [safe](https://example.com) [encoded](javascript%3aalert(1)) <test@example.com>",
        );

        assert!(!rendered.contains("mailto:"));
        assert!(rendered.contains("https://example.com"));
        assert!(!rendered.contains("javascript%253aalert"));
        assert!(!rendered.contains("test@example.com</a>"));
    }

    fn entry(id: &str, role: EntryRole, text: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: id.to_string(),
            role,
            text: text.to_string(),
        }
    }
}
