use leptos::prelude::*;

use crate::presentation::auth_page::{
    AuthPageKind, AuthPageState, auth_page_view, initialize_auth_page, submit_credentials_handler,
};

#[component]
pub fn RegisterPage() -> impl IntoView {
    let state = AuthPageState::new();
    initialize_auth_page(AuthPageKind::Register, state);
    auth_page_view(
        AuthPageKind::Register,
        state,
        submit_credentials_handler(AuthPageKind::Register, state),
    )
}
