use super::*;

#[test]
fn composer_cursor_position_uses_display_width_for_unicode_input() {
    let mut app = ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![]);
    app.insert_char('é');
    app.insert_char('界');

    assert_eq!(app.cursor(), 5);
    assert_eq!(app.cursor_display_width(), 3);
    assert_eq!(
        render::composer_cursor_position(Rect::new(0, 0, 20, 3), &app),
        (4, 1)
    );
}
