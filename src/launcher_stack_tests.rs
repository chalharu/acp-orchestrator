use super::*;
use acp_app_support::unique_temp_json_path;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn reusable_launcher_backend_url_accepts_live_backend_and_mock() {
    let (backend_url, health_task) = spawn_health_server().await;
    let mock_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock listener should bind");
    let state_path = unique_temp_json_path("acp-launcher-state", "healthy-stack");
    fs::write(
        &state_path,
        format!(
            "{{\"backend_url\":\"{backend_url}\",\"mock_address\":\"{}\"}}",
            mock_listener
                .local_addr()
                .expect("mock listener address should be readable")
        ),
    )
    .expect("launcher state should be writable");

    assert_eq!(
        reusable_launcher_backend_url(&state_path)
            .await
            .expect("healthy launcher state should be reusable"),
        Some(backend_url.clone())
    );

    health_task.abort();
}

#[tokio::test]
async fn reusable_launcher_backend_url_rejects_dead_mock_endpoints() {
    let (backend_url, health_task) = spawn_health_server().await;
    let state_path = unique_temp_json_path("acp-launcher-state", "dead-mock");
    fs::write(
        &state_path,
        format!("{{\"backend_url\":\"{backend_url}\",\"mock_address\":\"127.0.0.1:9\"}}"),
    )
    .expect("launcher state should be writable");

    assert_eq!(
        reusable_launcher_backend_url(&state_path)
            .await
            .expect("dead mock endpoints should not be reused"),
        None
    );

    health_task.abort();
}

#[test]
fn launcher_lock_path_appends_a_lock_suffix() {
    let path = launcher_lock_path_from(Path::new("/tmp/acp-launcher-state.json"));

    assert_eq!(path, PathBuf::from("/tmp/acp-launcher-state.json.lock"));
}

#[test]
fn launcher_lock_is_exclusive_until_it_is_dropped() {
    let lock_path =
        launcher_lock_path_from(&unique_temp_json_path("acp-launcher-lock", "exclusive"));
    let first_lock = try_acquire_launcher_lock(&lock_path)
        .expect("the first lock attempt should succeed")
        .expect("the first lock should be acquired");

    assert!(
        try_acquire_launcher_lock(&lock_path)
            .expect("the second lock attempt should return cleanly")
            .is_none()
    );
    drop(first_lock);

    let second_lock = try_acquire_launcher_lock(&lock_path)
        .expect("a released lock should be reusable")
        .expect("the released lock should be acquired");
    drop(second_lock);
}

#[test]
fn clear_stale_launcher_lock_removes_old_lock_files() {
    let lock_path = launcher_lock_path_from(&unique_temp_json_path("acp-launcher-lock", "stale"));
    fs::write(&lock_path, []).expect("stale launcher lock files should be creatable");

    assert!(
        clear_stale_launcher_lock(&lock_path, Duration::ZERO)
            .expect("stale launcher locks should be removable")
    );
    assert!(!lock_path.exists());
}

async fn spawn_health_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("health listener should bind");
    let address = listener
        .local_addr()
        .expect("health listener address should be readable");
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}",
                    )
                    .await;
            });
        }
    });

    (format!("http://{address}"), task)
}
