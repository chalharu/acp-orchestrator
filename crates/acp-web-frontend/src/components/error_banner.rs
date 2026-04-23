//! Error banner component.

use leptos::prelude::*;

#[component]
pub(crate) fn ErrorBanner(#[prop(into)] message: Signal<Option<String>>) -> impl IntoView {
    view! {
        <Show when=move || message.get().is_some()>
            <div class="banner" role="alert">
                <p class="banner__body">{move || message.get().unwrap_or_default()}</p>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::*;

    #[test]
    fn error_banner_builds_for_present_and_missing_messages() {
        let owner = Owner::new();
        owner.with(|| {
            let message = RwSignal::new(None::<String>);
            let derived = Signal::derive(move || message.get());

            let _ = view! { <ErrorBanner message=derived /> };
            message.set(Some("boom".to_string()));
            let _ = view! { <ErrorBanner message=derived /> };
        });
    }
}
