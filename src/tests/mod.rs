use super::support::frontend::{FrontendBundleAsset, frontend_bundle_file_name};
use super::support::temp::unique_temp_json_path;
use super::*;
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
    sync::{Mutex, MutexGuard},
};

mod args;
mod frontend_bundle;
mod process_roles;
mod web_launcher;

static TEST_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn test_env_lock() -> &'static Mutex<()> {
    TEST_ENV_LOCK.get_or_init(|| Mutex::const_new(()))
}

pub(crate) struct TestAcpServerUrlGuard {
    previous: Option<OsString>,
}

impl Drop for TestAcpServerUrlGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            unsafe {
                std::env::set_var("ACP_SERVER_URL", previous);
            }
        } else {
            unsafe {
                std::env::remove_var("ACP_SERVER_URL");
            }
        }
    }
}

pub(crate) fn test_acp_server_url_guard(value: Option<&str>) -> TestAcpServerUrlGuard {
    let previous = std::env::var_os("ACP_SERVER_URL");
    if let Some(value) = value {
        unsafe {
            std::env::set_var("ACP_SERVER_URL", value);
        }
    } else {
        unsafe {
            std::env::remove_var("ACP_SERVER_URL");
        }
    }
    TestAcpServerUrlGuard { previous }
}

struct TestPathGuard {
    previous: Option<OsString>,
}

impl Drop for TestPathGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            unsafe {
                std::env::set_var("PATH", previous);
            }
        } else {
            unsafe {
                std::env::remove_var("PATH");
            }
        }
    }
}

fn test_path_guard(value: Option<OsString>) -> TestPathGuard {
    let previous = std::env::var_os("PATH");
    if let Some(value) = value {
        unsafe {
            std::env::set_var("PATH", value);
        }
    } else {
        unsafe {
            std::env::remove_var("PATH");
        }
    }
    TestPathGuard { previous }
}

fn lock_acp_server_url() -> MutexGuard<'static, ()> {
    test_env_lock().blocking_lock()
}

async fn lock_acp_server_url_async() -> MutexGuard<'static, ()> {
    test_env_lock().lock().await
}

async fn spawn_single_response_http_server(
    response: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP listener should bind");
    let address = listener
        .local_addr()
        .expect("HTTP listener should expose its address");
    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    (format!("http://{address}"), handle)
}

async fn spawn_sleep_child() -> tokio::process::Child {
    Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .spawn()
        .expect("sleep child should spawn")
}

fn write_stub_frontend_bundle_assets(dist: &Path) -> Vec<PathBuf> {
    let tag = uuid::Uuid::new_v4();
    let javascript = dist.join(frontend_bundle_file_name(
        &tag.to_string(),
        FrontendBundleAsset::JavaScript,
    ));
    let wasm = dist.join(frontend_bundle_file_name(
        &tag.to_string(),
        FrontendBundleAsset::Wasm,
    ));
    fs::write(&javascript, "export default async function init() {}\n")
        .expect("stub javascript bundle should write");
    fs::write(&wasm, b"\x00asm\x01\x00\x00\x00").expect("stub wasm bundle should write");
    vec![javascript, wasm]
}

fn write_temp_frontend_root(label: &str) -> PathBuf {
    let root = unique_temp_json_path("acp-web-frontend-root", label).with_extension("");
    fs::create_dir_all(root.join("src")).expect("temp frontend src directory should be creatable");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"temp-frontend\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("temp frontend Cargo.toml should write");
    fs::write(root.join("Trunk.toml"), "dist = \"dist\"\n")
        .expect("temp frontend Trunk.toml should write");
    fs::write(
        root.join("index.html"),
        "<!doctype html>\n<div id=\"app-root\"></div>\n",
    )
    .expect("temp frontend index.html should write");
    fs::write(root.join("src").join("lib.rs"), "pub fn app() {}\n")
        .expect("temp frontend source should write");
    root
}

async fn prepare_managed_web_frontend_dist() -> Result<Option<PathBuf>> {
    prepare_frontend_dist(
        &LauncherArgs {
            acp_server: None,
            web: true,
            cli_args: Vec::new(),
        },
        true,
        false,
    )
    .await
}

fn write_fake_trunk_bin(command: &str) -> PathBuf {
    write_fake_trunk_bin_with_permissions(command, 0o755)
}

fn write_fake_trunk_bin_with_permissions(command: &str, mode: u32) -> PathBuf {
    let dir = unique_temp_json_path("acp-trunk-bin", "frontend-build").with_extension("");
    fs::create_dir_all(&dir).expect("fake trunk bin dir should be creatable");
    let trunk = dir.join("trunk");
    fs::write(&trunk, format!("#!/bin/sh\n{command}\n")).expect("fake trunk should write");
    let mut permissions = fs::metadata(&trunk)
        .expect("fake trunk metadata should load")
        .permissions();
    permissions.set_mode(mode);
    fs::set_permissions(&trunk, permissions).expect("fake trunk should become executable");
    dir
}

fn path_with_fake_trunk(front: &Path) -> OsString {
    std::env::join_paths([front, Path::new("/bin"), Path::new("/usr/bin")])
        .expect("PATH entries should be joinable")
}

fn path_without_trunk() -> OsString {
    std::env::join_paths([Path::new("/bin"), Path::new("/usr/bin")])
        .expect("PATH entries should be joinable")
}
