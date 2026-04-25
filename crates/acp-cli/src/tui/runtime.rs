use std::io;

use ratatui::{Terminal, backend::CrosstermBackend};
use reqwest::Client;
use snafu::ResultExt;
use tokio::{runtime::Handle, sync::mpsc};

use crate::contract_slash::CompletionCandidate;

use super::{
    TuiEvent,
    app::ChatApp,
    input::{UiContext, handle_terminal_event as handle_input_event},
    render,
};
use crate::{ChatSession, DrawTerminalUiSnafu, Result};

mod permissions;
mod terminal;

use permissions::{PendingPermissionRefreshState, drain_events, launch_pending_permission_refresh};
use terminal::read_terminal_event;
pub(crate) use terminal::run_terminal_ui;

pub(super) struct UiRunState {
    client: Client,
    server_url: String,
    auth_token: String,
    chat_session: ChatSession,
    command_catalog: Vec<CompletionCandidate>,
    startup_statuses: Vec<String>,
    event_tx: mpsc::UnboundedSender<TuiEvent>,
    event_rx: mpsc::UnboundedReceiver<TuiEvent>,
}

pub(super) struct UiEventChannel {
    pub(super) tx: mpsc::UnboundedSender<TuiEvent>,
    pub(super) rx: mpsc::UnboundedReceiver<TuiEvent>,
}

impl UiRunState {
    pub(super) fn new(
        client: Client,
        server_url: String,
        auth_token: String,
        chat_session: ChatSession,
        command_catalog: Vec<CompletionCandidate>,
        startup_statuses: Vec<String>,
        events: UiEventChannel,
    ) -> Self {
        Self {
            client,
            server_url,
            auth_token,
            chat_session,
            command_catalog,
            startup_statuses,
            event_tx: events.tx,
            event_rx: events.rx,
        }
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime_handle: Handle,
    state: UiRunState,
) -> Result<()> {
    let UiRunState {
        client,
        server_url,
        auth_token,
        chat_session,
        command_catalog,
        startup_statuses,
        event_tx,
        mut event_rx,
    } = state;

    let mut app = build_chat_app(
        &chat_session,
        &server_url,
        command_catalog,
        startup_statuses,
    );
    let context = UiContext {
        runtime_handle: &runtime_handle,
        client: &client,
        server_url: &server_url,
        auth_token: &auth_token,
        session_id: &chat_session.session.id,
    };
    let mut refresh_state = PendingPermissionRefreshState::default();

    loop {
        draw_app(
            terminal,
            &context,
            &event_tx,
            &mut event_rx,
            &mut app,
            &mut refresh_state,
        )?;
        if app.should_quit() {
            return Ok(());
        }

        if let Some(terminal_event) = read_terminal_event()? {
            let terminal_size = terminal.size().context(DrawTerminalUiSnafu)?;
            handle_input_event(terminal_size, &context, &mut app, terminal_event)?;
        }
    }
}

fn build_chat_app(
    chat_session: &ChatSession,
    server_url: &str,
    command_catalog: Vec<CompletionCandidate>,
    startup_statuses: Vec<String>,
) -> ChatApp {
    let mut app = ChatApp::new(
        &chat_session.session.id,
        server_url,
        chat_session.resumed,
        &chat_session.resume_history,
        &chat_session.session.pending_permissions,
        command_catalog,
    );
    for status in startup_statuses {
        app.push_status(status);
    }
    app
}

fn draw_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    context: &UiContext<'_>,
    event_tx: &mpsc::UnboundedSender<TuiEvent>,
    event_rx: &mut mpsc::UnboundedReceiver<TuiEvent>,
    app: &mut ChatApp,
    refresh_state: &mut PendingPermissionRefreshState,
) -> Result<()> {
    drain_events(event_rx, app, refresh_state);
    launch_pending_permission_refresh(context, event_tx, refresh_state);
    terminal
        .draw(|frame| render::render(frame, app))
        .context(DrawTerminalUiSnafu)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract_messages::{ConversationMessage, MessageRole};
    use crate::contract_permissions::PermissionRequest;
    use crate::contract_sessions::{CreateSessionResponse, SessionSnapshot, SessionStatus};
    use chrono::Utc;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn pending_permission_app() -> ChatApp {
        ChatApp::new(
            "s_test",
            "http://127.0.0.1:8080",
            false,
            &[],
            &[PermissionRequest {
                request_id: "req_1".to_string(),
                summary: "read_text_file README.md".to_string(),
            }],
            vec![],
        )
    }

    fn permissionless_app() -> ChatApp {
        ChatApp::new("s_test", "http://127.0.0.1:8080", false, &[], &[], vec![])
    }

    fn conversation_message(id: &str, role: MessageRole, text: &str) -> ConversationMessage {
        ConversationMessage {
            id: id.to_string(),
            role,
            text: text.to_string(),
            created_at: Utc::now(),
        }
    }

    fn session_response(pending_permissions: Vec<PermissionRequest>) -> CreateSessionResponse {
        CreateSessionResponse {
            session: SessionSnapshot {
                id: "s_test".to_string(),
                workspace_id: "w_test".to_string(),
                title: "New chat".to_string(),
                status: SessionStatus::Active,
                latest_sequence: 2,
                messages: Vec::new(),
                pending_permissions,
            },
        }
    }

    async fn spawn_session_server(response: CreateSessionResponse) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("server should bind");
        let address = listener
            .local_addr()
            .expect("server address should be readable");
        let payload = serde_json::to_vec(&response).expect("session payload should serialize");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response headers should write");
            stream
                .write_all(&payload)
                .await
                .expect("response body should write");
        });

        format!("http://{address}")
    }

    fn refresh_context<'a>(
        runtime_handle: &'a Handle,
        client: &'a Client,
        server_url: &'a str,
    ) -> UiContext<'a> {
        UiContext {
            runtime_handle,
            client,
            server_url,
            auth_token: "developer",
            session_id: "s_test",
        }
    }

    #[test]
    fn drain_events_requests_pending_permission_refresh_after_status_updates() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState::default();
        event_tx
            .send(TuiEvent::Stream(crate::events::StreamUpdate::Status(
                "turn cancelled".to_string(),
            )))
            .expect("status updates should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);
        assert!(refresh_state.queued);
        assert!(
            app.status_entries()
                .iter()
                .any(|status| status == "turn cancelled")
        );
    }

    #[test]
    fn drain_events_applies_refreshed_pending_permissions() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState {
            in_flight: true,
            queued: false,
        };
        event_tx
            .send(TuiEvent::PendingPermissionsRefreshed(Ok(Vec::new())))
            .expect("refreshed permissions should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);

        assert!(app.pending_permissions().is_empty());
        assert!(!refresh_state.in_flight);
    }

    #[test]
    fn drain_events_marks_the_connection_lost_when_the_stream_ends() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState::default();
        event_tx
            .send(TuiEvent::StreamEnded("event stream ended".to_string()))
            .expect("stream endings should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);

        assert_eq!(app.connection().label(), "disconnected");
        assert_eq!(app.connection().detail(), Some("event stream ended"));
    }

    #[test]
    fn drain_events_skips_refreshes_when_no_pending_permissions_exist() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = permissionless_app();
        let mut refresh_state = PendingPermissionRefreshState::default();
        event_tx
            .send(TuiEvent::Stream(crate::events::StreamUpdate::Status(
                "assistant finished".to_string(),
            )))
            .expect("status updates should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);

        assert!(!refresh_state.queued);
    }

    #[test]
    fn drain_events_requests_refresh_after_assistant_messages() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState::default();
        event_tx
            .send(TuiEvent::Stream(
                crate::events::StreamUpdate::ConversationMessage(conversation_message(
                    "m_assistant",
                    MessageRole::Assistant,
                    "done",
                )),
            ))
            .expect("assistant messages should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);

        assert!(refresh_state.queued);
    }

    #[test]
    fn drain_events_does_not_refresh_after_permission_requests() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState::default();
        event_tx
            .send(TuiEvent::Stream(
                crate::events::StreamUpdate::PermissionRequested(PermissionRequest {
                    request_id: "req_2".to_string(),
                    summary: "write_file Cargo.toml".to_string(),
                }),
            ))
            .expect("permission requests should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);

        assert!(!refresh_state.queued);
    }

    #[test]
    fn drain_events_records_refresh_errors() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut app = pending_permission_app();
        let mut refresh_state = PendingPermissionRefreshState {
            in_flight: true,
            queued: false,
        };
        event_tx
            .send(TuiEvent::PendingPermissionsRefreshed(Err(
                "refresh failed".to_string()
            )))
            .expect("refresh errors should queue");

        drain_events(&mut event_rx, &mut app, &mut refresh_state);

        assert!(
            app.status_entries()
                .iter()
                .any(|status| status == "refresh failed")
        );
        assert!(!refresh_state.in_flight);
    }

    #[tokio::test]
    async fn launch_pending_permission_refresh_ignores_unqueued_requests() {
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let client = Client::builder().build().expect("client should build");
        let runtime_handle = Handle::current();
        let mut refresh_state = PendingPermissionRefreshState::default();
        let context = refresh_context(&runtime_handle, &client, "http://127.0.0.1:9");

        launch_pending_permission_refresh(&context, &event_tx, &mut refresh_state);

        assert!(!refresh_state.in_flight);
        assert!(matches!(
            event_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn launch_pending_permission_refresh_fetches_latest_permissions() {
        let server_url = spawn_session_server(session_response(Vec::new())).await;
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let client = Client::builder().build().expect("client should build");
        let runtime_handle = Handle::current();
        let mut refresh_state = PendingPermissionRefreshState {
            in_flight: false,
            queued: true,
        };
        let context = refresh_context(&runtime_handle, &client, &server_url);

        launch_pending_permission_refresh(&context, &event_tx, &mut refresh_state);
        assert!(refresh_state.in_flight);
        assert!(!refresh_state.queued);
        match event_rx.recv().await.expect("refresh result should arrive") {
            TuiEvent::PendingPermissionsRefreshed(Ok(pending_permissions)) => {
                assert!(pending_permissions.is_empty());
            }
            other => panic!("unexpected refresh event: {other:?}"),
        }
    }
}
