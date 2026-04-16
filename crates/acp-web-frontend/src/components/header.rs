//! Page header with status grid.

use leptos::prelude::*;

#[component]
pub fn Header(
    backend_origin: String,
    #[prop(into)] connection_status: Signal<String>,
    #[prop(into)] session_status: Signal<String>,
    #[prop(into)] route_summary: String,
) -> impl IntoView {
    view! {
        <header class="app-header">
            <div class="app-header__copy">
                <p class="eyebrow">"ACP Web MVP slice 1"</p>
                <h1>"ACP Web chat"</h1>
                <p class="muted">{route_summary}</p>
            </div>
            <dl class="status-grid" aria-label="session status">
                <div>
                    <dt>"Backend"</dt>
                    <dd>{backend_origin}</dd>
                </div>
                <div>
                    <dt>"Connection"</dt>
                    <dd>{move || connection_status.get()}</dd>
                </div>
                <div>
                    <dt>"Session"</dt>
                    <dd>{move || session_status.get()}</dd>
                </div>
            </dl>
        </header>
    }
}
