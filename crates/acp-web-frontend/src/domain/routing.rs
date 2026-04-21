use crate::api;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AppRoute {
    Home,
    Register,
    Session(String),
    NotFound,
}

pub(crate) fn current_route() -> AppRoute {
    let Some(pathname) = web_sys::window().and_then(|window| window.location().pathname().ok())
    else {
        return AppRoute::NotFound;
    };

    route_from_pathname(&pathname)
}

pub(crate) fn route_from_pathname(pathname: &str) -> AppRoute {
    if pathname == "/app" || pathname == "/app/" {
        return AppRoute::Home;
    }
    if pathname == "/app/register" || pathname == "/app/register/" {
        return AppRoute::Register;
    }

    pathname
        .strip_prefix("/app/sessions/")
        .filter(|session_id| !session_id.is_empty())
        .and_then(api::decode_component)
        .map(AppRoute::Session)
        .unwrap_or(AppRoute::NotFound)
}

pub(crate) fn app_session_path(session_id: &str) -> String {
    format!("/app/sessions/{}", api::encode_component(session_id))
}
