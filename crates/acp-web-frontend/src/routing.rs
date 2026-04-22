#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AppRoute {
    Home,
    Register,
    SignIn,
    Accounts,
    Session(String),
    NotFound,
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn current_route() -> AppRoute {
    let Some(pathname) = web_sys::window().and_then(|window| window.location().pathname().ok())
    else {
        return AppRoute::NotFound;
    };

    route_from_pathname(&pathname)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn current_route() -> AppRoute {
    AppRoute::NotFound
}

pub(crate) fn route_from_pathname(pathname: &str) -> AppRoute {
    if pathname == "/app" || pathname == "/app/" {
        return AppRoute::Home;
    }
    if pathname == "/app/register" || pathname == "/app/register/" {
        return AppRoute::Register;
    }
    if pathname == "/app/sign-in" || pathname == "/app/sign-in/" {
        return AppRoute::SignIn;
    }
    if pathname == "/app/accounts" || pathname == "/app/accounts/" {
        return AppRoute::Accounts;
    }

    pathname
        .strip_prefix("/app/sessions/")
        .filter(|session_id| !session_id.is_empty())
        .and_then(decode_component)
        .map(AppRoute::Session)
        .unwrap_or(AppRoute::NotFound)
}

pub(crate) fn app_session_path(session_id: &str) -> String {
    format!("/app/sessions/{}", encode_component(session_id))
}

pub(crate) fn encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

pub(crate) fn decode_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }

        let high = *bytes.get(index + 1)?;
        let low = *bytes.get(index + 2)?;
        decoded.push((hex_value(high)? << 4) | hex_value(low)?);
        index += 3;
    }

    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppRoute, app_session_path, current_route, decode_component, encode_component,
        route_from_pathname,
    };

    #[test]
    fn app_session_path_encodes_reserved_session_id_characters() {
        assert_eq!(app_session_path("s/1"), "/app/sessions/s%2F1");
    }

    #[test]
    fn route_from_pathname_decodes_session_id_segments() {
        assert_eq!(route_from_pathname("/app/register"), AppRoute::Register);
        assert_eq!(route_from_pathname("/app/register/"), AppRoute::Register);
        assert_eq!(route_from_pathname("/app/sign-in"), AppRoute::SignIn);
        assert_eq!(route_from_pathname("/app/sign-in/"), AppRoute::SignIn);
        assert_eq!(route_from_pathname("/app/accounts"), AppRoute::Accounts);
        assert_eq!(route_from_pathname("/app/accounts/"), AppRoute::Accounts);
        assert_eq!(
            route_from_pathname("/app/sessions/s%2F1"),
            AppRoute::Session("s/1".to_string())
        );
        assert_eq!(route_from_pathname("/app/sessions/%ZZ"), AppRoute::NotFound);
    }

    #[test]
    fn component_encoding_round_trips_and_rejects_invalid_hex() {
        assert_eq!(encode_component("s/1"), "s%2F1");
        assert_eq!(decode_component("s%2F1"), Some("s/1".to_string()));
        assert_eq!(decode_component("s%2f1"), Some("s/1".to_string()));
        assert_eq!(
            decode_component("hello%20world"),
            Some("hello world".to_string())
        );
        assert_eq!(decode_component("%E3%81%82"), Some("あ".to_string()));
        assert_eq!(decode_component("%ZZ"), None);
        assert_eq!(decode_component("%A"), None);
    }

    #[test]
    fn route_from_pathname_handles_home_and_unknown_paths() {
        assert_eq!(route_from_pathname("/app"), AppRoute::Home);
        assert_eq!(route_from_pathname("/app/"), AppRoute::Home);
        assert_eq!(route_from_pathname("/app/sessions/"), AppRoute::NotFound);
        assert_eq!(route_from_pathname("/app/sessions"), AppRoute::NotFound);
        assert_eq!(route_from_pathname("/outside"), AppRoute::NotFound);
    }

    #[test]
    fn current_route_falls_back_to_not_found_without_a_browser_window() {
        assert_eq!(current_route(), AppRoute::NotFound);
    }
}
