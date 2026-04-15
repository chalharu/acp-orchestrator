use super::*;

#[test]
fn handle_key_edits_input_characters() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
    let client = Client::builder().build().expect("client should build");
    let server_url = "http://127.0.0.1:9".to_string();
    let auth_token = "developer".to_string();
    let session_id = "s_test".to_string();
    let context = build_context(
        runtime.handle(),
        &client,
        &server_url,
        &auth_token,
        &session_id,
    );
    let mut app = base_app();

    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        3,
        40,
    )
    .expect("editing should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("left should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("right should succeed");
    handle_key(
        &context,
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        3,
        40,
    )
    .expect("backspace should succeed");

    assert_eq!(app.input(), "");
}
