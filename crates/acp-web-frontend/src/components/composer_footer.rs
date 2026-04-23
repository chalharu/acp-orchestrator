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
}
