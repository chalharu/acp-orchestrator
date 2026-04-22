pub(crate) use crate::routing::{decode_component, encode_component};

pub(crate) fn session_path(session_id: &str) -> String {
    format!("/api/v1/sessions/{}", encode_component(session_id))
}

pub(crate) fn permission_url(session_id: &str, request_id: &str) -> String {
    format!(
        "{}/permissions/{}",
        session_path(session_id),
        encode_component(request_id),
    )
}

#[cfg(test)]
mod tests {
    use super::{decode_component, permission_url, session_path};

    #[test]
    fn session_path_encodes_special_characters() {
        assert_eq!(session_path("s_123"), "/api/v1/sessions/s_123");
        assert_eq!(session_path("s/1"), "/api/v1/sessions/s%2F1");
        assert_eq!(session_path("../../etc"), "/api/v1/sessions/..%2F..%2Fetc");
    }

    #[test]
    fn decode_component_decodes_percent_encoded_utf8() {
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
    fn permission_url_encodes_session_and_request_ids() {
        assert_eq!(
            permission_url("s/1", "../../close"),
            "/api/v1/sessions/s%2F1/permissions/..%2F..%2Fclose"
        );
    }
}
