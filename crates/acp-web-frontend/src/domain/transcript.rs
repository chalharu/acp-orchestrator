use pulldown_cmark::{
    Event as MarkdownEvent, LinkType, Options as MarkdownOptions, Parser, Tag, TagEnd,
    html::push_html,
};

const OVERSCAN_PX: f64 = 320.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TranscriptEntry {
    pub id: String,
    pub role: EntryRole,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VirtualWindow {
    pub visible: Vec<TranscriptEntry>,
    pub top_spacer_height: f64,
    pub bottom_spacer_height: f64,
}

pub(crate) fn render_markdown(source: &str) -> String {
    let parser = Parser::new_ext(source, markdown_options());
    let mut rendered = String::new();
    push_html(&mut rendered, sanitize_markdown_events(parser).into_iter());
    rendered
}

pub(crate) fn tail_scroll_top(scroll_height: i32, client_height: i32) -> i32 {
    (scroll_height - client_height).max(0)
}

pub(crate) fn compute_virtual_window(
    entries: &[TranscriptEntry],
    scroll_top: f64,
    viewport_height: f64,
) -> VirtualWindow {
    if entries.is_empty() {
        return empty_virtual_window();
    }

    let start_threshold = (scroll_top - OVERSCAN_PX).max(0.0);
    let end_threshold = scroll_top + viewport_height + OVERSCAN_PX;
    let (start_index, top_spacer_height) = find_window_start(entries, start_threshold);
    let (end_index, rendered_height) =
        find_window_end(entries, start_index, top_spacer_height, end_threshold);

    let total_height = entries.iter().map(estimated_entry_height).sum::<f64>();

    VirtualWindow {
        visible: entries[start_index..end_index].to_vec(),
        top_spacer_height,
        bottom_spacer_height: (total_height - rendered_height).max(0.0),
    }
}

fn empty_virtual_window() -> VirtualWindow {
    VirtualWindow {
        visible: Vec::new(),
        top_spacer_height: 0.0,
        bottom_spacer_height: 0.0,
    }
}

fn find_window_start(entries: &[TranscriptEntry], start_threshold: f64) -> (usize, f64) {
    let mut top_spacer_height = 0.0;
    let mut start_index = 0;

    while start_index < entries.len() {
        let next_offset = top_spacer_height + estimated_entry_height(&entries[start_index]);
        if next_offset >= start_threshold {
            break;
        }
        top_spacer_height = next_offset;
        start_index += 1;
    }

    if start_index == entries.len() {
        start_index = entries.len().saturating_sub(1);
        top_spacer_height -= estimated_entry_height(&entries[start_index]);
    }

    (start_index, top_spacer_height)
}

fn find_window_end(
    entries: &[TranscriptEntry],
    start_index: usize,
    top_spacer_height: f64,
    end_threshold: f64,
) -> (usize, f64) {
    let mut end_index = start_index;
    let mut rendered_height = top_spacer_height;

    while end_index < entries.len() {
        rendered_height += estimated_entry_height(&entries[end_index]);
        end_index += 1;
        if rendered_height >= end_threshold {
            break;
        }
    }

    (end_index, rendered_height)
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
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => Some(MarkdownEvent::HardBreak),
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
        Tag::Heading { level, .. } => Some(MarkdownEvent::Start(Tag::Heading {
            level,
            id: None,
            classes: Vec::new(),
            attrs: Vec::new(),
        })),
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
        tag => Some(MarkdownEvent::Start(tag)),
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
        TagEnd::HtmlBlock
        | TagEnd::Image
        | TagEnd::FootnoteDefinition
        | TagEnd::MetadataBlock(_) => None,
        tag_end => Some(MarkdownEvent::End(tag_end)),
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
    use pulldown_cmark::{
        CowStr, Event as MarkdownEvent, HeadingLevel, LinkType, MetadataBlockKind,
        Options as MarkdownOptions, Parser, Tag, TagEnd,
    };

    use super::{
        EntryRole, TranscriptEntry, compute_virtual_window, estimate_visual_lines,
        estimated_entry_height, is_safe_markdown_link, is_safe_markdown_url, markdown_options,
        render_markdown, sanitize_markdown_end, sanitize_markdown_events, sanitize_markdown_start,
        tail_scroll_top,
    };

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

    #[test]
    fn virtual_window_returns_all_entries_for_short_transcripts() {
        let entries = vec![entry("1", EntryRole::Assistant, "hello")];
        let window = compute_virtual_window(&entries, 0.0, 800.0);

        assert_eq!(window.visible, entries);
        assert_eq!(window.top_spacer_height, 0.0);
        assert_eq!(window.bottom_spacer_height, 0.0);
    }

    #[test]
    fn virtual_window_handles_empty_and_past_end_scroll_positions() {
        let empty = compute_virtual_window(&[], 40.0, 120.0);
        assert!(empty.visible.is_empty());
        assert_eq!(empty.top_spacer_height, 0.0);
        assert_eq!(empty.bottom_spacer_height, 0.0);

        let entries = (0..3)
            .map(|index| entry(&index.to_string(), EntryRole::Status, "done"))
            .collect::<Vec<_>>();
        let window = compute_virtual_window(&entries, 5_000.0, 100.0);

        assert_eq!(window.visible, vec![entries[2].clone()]);
        assert!(window.top_spacer_height > 0.0);
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

    #[test]
    fn markdown_sanitizers_convert_special_events_into_safe_output_events() {
        let mut options = markdown_options();
        options.insert(MarkdownOptions::ENABLE_FOOTNOTES);
        options.insert(MarkdownOptions::ENABLE_MATH);
        let events = sanitize_markdown_events(Parser::new_ext(
            "line one\nline two\\\n$inline$ $$display$$ [^note]\n\n- [x] done\n\n---\n\n[^note]: footnote",
            options,
        ));

        assert!(events.contains(&MarkdownEvent::HardBreak));
        assert!(events.contains(&MarkdownEvent::Rule));
        assert!(events.contains(&MarkdownEvent::TaskListMarker(true)));
        assert!(events.iter().any(|event| matches!(
            event,
            MarkdownEvent::Text(text) if text.as_ref() == "inline"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            MarkdownEvent::Text(text) if text.as_ref() == "display"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            MarkdownEvent::Text(text) if text.as_ref() == "note"
        )));
    }

    #[test]
    fn markdown_sanitizers_strip_heading_metadata() {
        let mut ignored_ends = Vec::new();
        let heading = sanitize_markdown_start(
            Tag::Heading {
                level: HeadingLevel::H2,
                id: Some(CowStr::from("keep-out")),
                classes: vec![CowStr::from("fancy")],
                attrs: vec![(CowStr::from("data-x"), Some(CowStr::from("1")))],
            },
            &mut ignored_ends,
        );
        assert!(matches!(
            heading,
            Some(pulldown_cmark::Event::Start(Tag::Heading {
                level: HeadingLevel::H2,
                id: None,
                ..
            }))
        ));
        assert!(ignored_ends.is_empty());
    }

    fn assert_start_tag_ignored(tag: Tag<'static>, expected_end: TagEnd) {
        let mut ignored_ends = Vec::new();
        assert!(sanitize_markdown_start(tag, &mut ignored_ends).is_none());
        assert_eq!(ignored_ends.pop(), Some(expected_end));
    }

    fn assert_end_tag_ignored(tag_end: TagEnd) {
        assert!(sanitize_markdown_end(tag_end, &mut Vec::new()).is_none());
    }

    #[test]
    fn markdown_sanitizers_track_ignored_start_tags() {
        assert_start_tag_ignored(
            Tag::Link {
                link_type: LinkType::Inline,
                dest_url: CowStr::from("javascript:alert(1)"),
                title: CowStr::from(""),
                id: CowStr::from(""),
            },
            TagEnd::Link,
        );
        assert_start_tag_ignored(
            Tag::Image {
                link_type: LinkType::Inline,
                dest_url: CowStr::from("https://example.com/x.png"),
                title: CowStr::from("preview"),
                id: CowStr::from(""),
            },
            TagEnd::Image,
        );
        assert_start_tag_ignored(Tag::HtmlBlock, TagEnd::HtmlBlock);
        assert_start_tag_ignored(
            Tag::FootnoteDefinition(CowStr::from("note")),
            TagEnd::FootnoteDefinition,
        );
        assert_start_tag_ignored(
            Tag::MetadataBlock(MetadataBlockKind::YamlStyle),
            TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle),
        );
    }

    #[test]
    fn markdown_sanitizers_drop_special_end_tags_and_preserve_normal_ones() {
        assert_end_tag_ignored(TagEnd::Image);
        assert_end_tag_ignored(TagEnd::HtmlBlock);
        assert_end_tag_ignored(TagEnd::FootnoteDefinition);
        assert_end_tag_ignored(TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle));

        let mut ignored_link_end = vec![TagEnd::Link];
        assert!(sanitize_markdown_end(TagEnd::Link, &mut ignored_link_end).is_none());
        assert!(ignored_link_end.is_empty());

        assert!(matches!(
            sanitize_markdown_end(TagEnd::Paragraph, &mut Vec::new()),
            Some(pulldown_cmark::Event::End(TagEnd::Paragraph))
        ));
    }

    #[test]
    fn markdown_sanitizers_preserve_safe_links_and_reject_unsafe_link_types() {
        assert!(matches!(
            sanitize_markdown_start(
                Tag::Link {
                    link_type: LinkType::Inline,
                    dest_url: CowStr::from("https://example.com/docs"),
                    title: CowStr::from("docs"),
                    id: CowStr::from(""),
                },
                &mut Vec::new(),
            ),
            Some(pulldown_cmark::Event::Start(Tag::Link { .. }))
        ));
        assert!(!is_safe_markdown_link(
            LinkType::Email,
            "mailto:test@example.com"
        ));
        assert!(!is_safe_markdown_link(
            LinkType::WikiLink { has_pothole: false },
            "Guide"
        ));
    }

    #[test]
    fn safe_url_checks_allow_relative_paths_and_block_encoded_schemes() {
        assert!(is_safe_markdown_url("#anchor"));
        assert!(is_safe_markdown_url("/root"));
        assert!(is_safe_markdown_url("./child"));
        assert!(is_safe_markdown_url("../parent"));
        assert!(is_safe_markdown_url("https://example.com"));
        assert!(is_safe_markdown_url("docs/page"));
        assert!(!is_safe_markdown_url("javascript%3Aalert(1)"));
        assert!(!is_safe_markdown_url("javascript:alert(1)"));
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

    fn entry(id: &str, role: EntryRole, text: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: id.to_string(),
            role,
            text: text.to_string(),
        }
    }
}
