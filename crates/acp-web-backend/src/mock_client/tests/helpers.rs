use super::*;
use agent_client_protocol::{ConnectTo, HandleDispatchFrom, schema};
use std::collections::HashMap;

#[test]
fn reply_from_stop_reason_maps_cancelled_and_empty_outputs() {
    assert_eq!(
        reply_from_stop_reason(schema::StopReason::Cancelled, "ignored".to_string()),
        ReplyResult::Status("turn cancelled".to_string())
    );
    assert_eq!(
        reply_from_stop_reason(schema::StopReason::EndTurn, String::new()),
        ReplyResult::NoOutput
    );
    assert_eq!(
        reply_from_stop_reason(schema::StopReason::EndTurn, "reply".to_string()),
        ReplyResult::Reply("reply".to_string())
    );
}

#[test]
fn reuse_cached_session_keeps_successful_session_mappings() {
    let mut upstream_sessions = HashMap::from([("backend".to_string(), "mock_0".to_string())]);

    assert_eq!(
        reuse_cached_session(
            Some("mock_0".to_string()),
            true,
            &mut upstream_sessions,
            "backend",
        ),
        Some("mock_0".to_string())
    );
    assert_eq!(
        upstream_sessions.get("backend").map(String::as_str),
        Some("mock_0")
    );
}

#[test]
fn reuse_cached_session_clears_stale_mappings_after_failed_loads() {
    let mut upstream_sessions = HashMap::from([("backend".to_string(), "mock_0".to_string())]);

    assert_eq!(
        reuse_cached_session(
            Some("mock_0".to_string()),
            false,
            &mut upstream_sessions,
            "backend",
        ),
        None
    );
    assert!(
        !upstream_sessions.contains_key("backend"),
        "stale cached mappings should be removed"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn handle_cancelled_prompt_sends_cancels_and_reports_cancelled_status() {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let prompt_future = async { Ok(schema::PromptResponse::new(schema::StopReason::EndTurn)) };
    tokio::pin!(prompt_future);
    let client = BackendAcpClient::without_turn();

    let reply = handle_cancelled_prompt(
        true,
        &mut prompt_future,
        schema::CancelNotification::new("mock_0"),
        |cancel_request| {
            cancel_tx
                .send(cancel_request)
                .expect("cancel requests should send");
            Ok(())
        },
        &client,
    )
    .await
    .expect("cancelled prompts should resolve");

    assert_eq!(
        cancel_rx
            .await
            .expect("cancel requests should be received")
            .session_id
            .to_string(),
        "mock_0"
    );
    assert_eq!(reply, ReplyResult::Status("turn cancelled".to_string()));
}

#[tokio::test]
async fn backend_io_connect_to_completes_when_peer_closes() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("test listener should expose a local address");
    let (client_stream, accepted) =
        tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
    let client_stream = client_stream.expect("test client should connect");
    let (server_stream, _) = accepted.expect("test listener should accept");
    let (reader, writer) = server_stream.into_split();
    let (channel, counterpart) = acp::Channel::duplex();

    drop(channel);
    drop(client_stream);

    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        ConnectTo::<acp::Agent>::connect_to(BackendIo::new(reader, writer), counterpart),
    )
    .await
    .expect("BackendIo should stop promptly when the peer closes");
}

#[test]
fn backend_dispatch_handler_describes_itself() {
    let handler = BackendDispatchHandler::new(
        BackendAcpClient::without_turn(),
        BackendAcpClient::without_turn(),
    );

    assert_eq!(
        format!("{:?}", handler.describe_chain()),
        "\"BackendDispatchHandler\""
    );
}
