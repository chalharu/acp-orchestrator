use leptos::prelude::*;

use crate::presentation::auth_page::{
    AuthPageKind, AuthPageState, auth_page_view, initialize_auth_page, submit_credentials_handler,
};

#[component]
pub fn SignInPage() -> impl IntoView {
    let state = AuthPageState::new();
    initialize_auth_page(AuthPageKind::SignIn, state);
    auth_page_view(
        AuthPageKind::SignIn,
        state,
        submit_credentials_handler(AuthPageKind::SignIn, state),
    )
}
