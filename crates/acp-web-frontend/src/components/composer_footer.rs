//! Composer footer with actions and status display.

use leptos::prelude::*;

use crate::presentation::{AppIcon, app_icon_view};

#[component]
pub(super) fn ComposerFooter(
    #[prop(into)] status_text: Signal<String>,
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let status_binding = composer_status_binding(status_text);

    view! {
        <div class="composer__footer">
            <p class="composer__status" hidden=move || composer_status_hidden(status_text)>
                {status_binding}
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

fn composer_status_text(status_text: Signal<String>) -> String {
    status_text.get()
}

fn composer_status_hidden(status_text: Signal<String>) -> bool {
    composer_status_text(status_text).is_empty()
}

fn composer_status_binding(status_text: Signal<String>) -> impl Fn() -> String + Copy + 'static {
    move || composer_status_text(status_text)
}

#[component]
fn ComposerActions(
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    composer_actions_view(disabled, show_cancel, cancel_disabled, on_cancel)
}

#[rustfmt::skip]
fn composer_actions_view(
    disabled: Signal<bool>,
    show_cancel: Signal<bool>,
    cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let cancel_props = (cancel_disabled, on_cancel);
    view! { <div class="composer__actions"><Show when=move || show_cancel.get()><ComposerCancelButton cancel_disabled=cancel_props.0 on_cancel=cancel_props.1 /></Show><button class="composer__submit icon-action icon-action--primary" type="submit" prop:disabled=move || disabled.get() aria-label=submit_button_label() title=submit_button_label()>{app_icon_view(AppIcon::Send)}<span class="sr-only">{submit_button_label()}</span></button></div> }
}

#[component]
fn ComposerCancelButton(
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let cancel_class = cancel_button_class();
    let cancel_type = cancel_button_type();
    let cancel_label = cancel_button_label();
    let cancel_click = cancel_click_handler(on_cancel);
    let cancel_disabled_binding = cancel_button_disabled_binding(cancel_disabled);

    view! {
        <button
            class=cancel_class
            type=cancel_type
            on:click=cancel_click
            prop:disabled=cancel_disabled_binding
            aria-label=cancel_label
            title=cancel_label
        >
            {app_icon_view(AppIcon::Cancel)}
            <span class="sr-only">{cancel_label}</span>
        </button>
    }
}

fn cancel_button_class() -> &'static str {
    "composer__cancel icon-action icon-action--ghost"
}

fn cancel_button_type() -> &'static str {
    "button"
}

fn cancel_button_label() -> &'static str {
    "Cancel"
}

fn submit_button_label() -> &'static str {
    "Send"
}

fn cancel_click_handler<E>(on_cancel: Callback<()>) -> impl Fn(E) + Copy + 'static
where
    E: 'static,
{
    move |_event: E| run_cancel(on_cancel)
}

fn run_cancel(on_cancel: Callback<()>) {
    on_cancel.run(());
}

fn cancel_button_disabled(cancel_disabled: Signal<bool>) -> bool {
    cancel_disabled.get()
}

fn cancel_button_disabled_binding(
    cancel_disabled: Signal<bool>,
) -> impl Fn() -> bool + Copy + 'static {
    move || cancel_button_disabled(cancel_disabled)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn composer_cancel_button_builds_without_panicking() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = view! {
                <ComposerCancelButton
                    cancel_disabled=Signal::derive(|| false)
                    on_cancel=Callback::new(|()| {})
                />
            };
        });
    }

    #[test]
    fn composer_actions_view_renders_send_and_cancel_actions() {
        let owner = Owner::new();
        owner.with(|| {
            let _ = composer_actions_view(
                Signal::derive(|| false),
                Signal::derive(|| true),
                Signal::derive(|| false),
                Callback::new(|()| {}),
            )
            .into_any();
            assert_eq!(submit_button_label(), "Send");
            assert_eq!(cancel_button_label(), "Cancel");
        });
    }

    #[test]
    fn composer_footer_helpers_return_expected_values() {
        let owner = Owner::new();
        owner.with(|| {
            let ready = Signal::derive(|| "Ready".to_string());
            let empty = Signal::derive(String::new);

            assert_eq!(composer_status_text(ready), "Ready");
            assert!(!composer_status_hidden(ready));
            assert!(composer_status_hidden(empty));
            assert_eq!(composer_status_binding(ready)(), "Ready");
            assert_eq!(
                cancel_button_class(),
                "composer__cancel icon-action icon-action--ghost"
            );
            assert_eq!(cancel_button_type(), "button");
            assert_eq!(cancel_button_label(), "Cancel");
            assert_eq!(submit_button_label(), "Send");
            assert!(!cancel_button_disabled(Signal::derive(|| false)));
            assert!(cancel_button_disabled(Signal::derive(|| true)));
            assert!(!cancel_button_disabled_binding(Signal::derive(|| false))());
            assert!(cancel_button_disabled_binding(Signal::derive(|| true))());
        });
    }

    #[test]
    fn run_cancel_invokes_callback() {
        let owner = Owner::new();
        owner.with(|| {
            let called = RwSignal::new(false);

            run_cancel(Callback::new(move |()| called.set(true)));

            assert!(called.get_untracked());
        });
    }

    #[test]
    fn cancel_click_handler_invokes_callback() {
        let owner = Owner::new();
        owner.with(|| {
            let called = RwSignal::new(false);

            cancel_click_handler::<()>(Callback::new(move |()| called.set(true)))(());

            assert!(called.get_untracked());
        });
    }
}
