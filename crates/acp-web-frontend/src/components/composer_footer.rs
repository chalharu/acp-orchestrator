//! Composer footer with actions and status display.

use leptos::prelude::*;

#[component]
pub(super) fn ComposerFooter(
    #[prop(into)] status_text: Signal<String>,
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] show_cancel: Signal<bool>,
    #[prop(into)] cancel_disabled: Signal<bool>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="composer__footer">
            <p class="composer__status" hidden=move || composer_status_hidden(status_text)>
                {move || composer_status_text(status_text)}
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
                    class=cancel_button_class()
                    type=cancel_button_type()
                    on:click=move |_| run_cancel(on_cancel)
                    prop:disabled=move || cancel_button_disabled(cancel_disabled)
                >
                    {cancel_button_label()}
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

fn cancel_button_class() -> &'static str {
    "pending-list__button--secondary composer__cancel"
}

fn cancel_button_type() -> &'static str {
    "button"
}

fn cancel_button_label() -> &'static str {
    "Cancel"
}

fn run_cancel(on_cancel: Callback<()>) {
    on_cancel.run(());
}

fn cancel_button_disabled(cancel_disabled: Signal<bool>) -> bool {
    cancel_disabled.get()
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
    fn composer_footer_helpers_return_expected_values() {
        let owner = Owner::new();
        owner.with(|| {
            let ready = Signal::derive(|| "Ready".to_string());
            let empty = Signal::derive(String::new);

            assert_eq!(composer_status_text(ready), "Ready");
            assert!(!composer_status_hidden(ready));
            assert!(composer_status_hidden(empty));
            assert_eq!(
                cancel_button_class(),
                "pending-list__button--secondary composer__cancel"
            );
            assert_eq!(cancel_button_type(), "button");
            assert_eq!(cancel_button_label(), "Cancel");
            assert!(!cancel_button_disabled(Signal::derive(|| false)));
            assert!(cancel_button_disabled(Signal::derive(|| true)));
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
}
