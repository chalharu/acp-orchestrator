//! Error banner component.

use leptos::prelude::*;

#[component]
pub fn ErrorBanner(message: RwSignal<Option<String>>) -> impl IntoView {
    view! {
        <Show when=move || message.get().is_some()>
            <div class="banner" role="alert">
                <p class="banner__body">{move || message.get().unwrap_or_default()}</p>
            </div>
        </Show>
    }
}
