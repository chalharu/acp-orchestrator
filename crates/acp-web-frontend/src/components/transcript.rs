//! Conversation transcript component.

use leptos::{html as leptos_html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure};

use crate::domain::transcript::{
    EntryRole, TranscriptEntry, compute_virtual_window, render_markdown, tail_scroll_top,
};

const DEFAULT_VIEWPORT_HEIGHT: f64 = 640.0;
const BOTTOM_TOLERANCE_PX: f64 = 24.0;

#[component]
pub(crate) fn Transcript(#[prop(into)] entries: Signal<Vec<TranscriptEntry>>) -> impl IntoView {
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
