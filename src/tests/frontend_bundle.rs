use super::*;

#[tokio::test]
async fn prepare_frontend_dist_skips_external_backend_launches() {
    let _guard = test_env_lock().lock().await;
    let _url_guard = test_acp_server_url_guard(Some("https://127.0.0.1:9443"));

    let frontend_dist = prepare_managed_web_frontend_dist()
        .await
        .expect("external backend launches should skip the managed frontend build");

    assert_eq!(frontend_dist, None);
}

#[tokio::test]
async fn prepare_frontend_dist_returns_workspace_dist_for_managed_web_launches() {
    let _guard = test_env_lock().lock().await;
    let _url_guard = test_acp_server_url_guard(None);
    let dist = frontend_dist_path();
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");
    let created_assets = write_stub_frontend_bundle_assets(&dist);

    let frontend_dist = prepare_managed_web_frontend_dist()
        .await
        .expect("managed web launches should prepare the frontend dist");

    assert_eq!(frontend_dist.as_deref(), Some(dist.as_path()));

    for asset in created_assets {
        let _ = fs::remove_file(asset);
    }
}

#[tokio::test]
async fn ensure_frontend_built_reports_missing_trunk() {
    let _guard = test_env_lock().lock().await;
    let _path_guard = test_path_guard(Some(path_without_trunk()));
    let dist = unique_temp_json_path("acp-frontend-dist", "missing-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    let error = ensure_frontend_built(&dist)
        .await
        .expect_err("missing trunk executables should be surfaced");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message }
            if message == "trunk not found – install it with `cargo install trunk`"
    ));
}

#[tokio::test]
async fn ensure_frontend_built_surfaces_failed_trunk_exit_codes() {
    let _guard = test_env_lock().lock().await;
    let fake_trunk_dir = write_fake_trunk_bin("exit 9");
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));
    let dist = unique_temp_json_path("acp-frontend-dist", "failed-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    let error = ensure_frontend_built(&dist)
        .await
        .expect_err("failed trunk builds should be surfaced");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message }
            if message == "`trunk build --release` failed with exit code Some(9)"
    ));
}

#[tokio::test]
async fn ensure_frontend_built_surfaces_other_trunk_spawn_errors() {
    let _guard = test_env_lock().lock().await;
    let fake_trunk_dir = write_fake_trunk_bin_with_permissions("exit 0", 0o644);
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));
    let dist = unique_temp_json_path("acp-frontend-dist", "unexecutable-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    let error = ensure_frontend_built(&dist)
        .await
        .expect_err("other trunk spawn failures should be surfaced");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message } if !message.is_empty()
    ));
}

#[tokio::test]
async fn ensure_frontend_built_accepts_successful_trunk_runs() {
    let _guard = test_env_lock().lock().await;
    let fake_trunk_dir = write_fake_trunk_bin("exit 0");
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));
    let dist = unique_temp_json_path("acp-frontend-dist", "successful-trunk").with_extension("");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");

    ensure_frontend_built(&dist)
        .await
        .expect("successful trunk builds should be accepted");
}

#[tokio::test]
async fn ensure_frontend_built_skips_trunk_when_dist_is_current() {
    let _guard = test_env_lock().lock().await;
    let fake_trunk_dir = write_fake_trunk_bin("exit 9");
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));
    let frontend_root = write_temp_frontend_root("fresh-dist");
    let dist = frontend_root.join("dist");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");
    std::thread::sleep(Duration::from_millis(20));
    write_stub_frontend_bundle_assets(&dist);

    ensure_frontend_built_at(&frontend_root, &dist)
        .await
        .expect("fresh frontend dist should skip rebuilding");
}

#[tokio::test]
async fn ensure_frontend_built_rebuilds_stale_dist() {
    let _guard = test_env_lock().lock().await;
    let frontend_root = write_temp_frontend_root("stale-dist");
    let dist = frontend_root.join("dist");
    fs::create_dir_all(&dist).expect("frontend dist directory should be creatable");
    write_stub_frontend_bundle_assets(&dist);
    std::thread::sleep(Duration::from_millis(20));
    fs::write(
        frontend_root.join("src").join("lib.rs"),
        "pub fn app() { let _ = 1; }\n",
    )
    .expect("frontend source should be writable");
    let marker = unique_temp_json_path("acp-trunk-marker", "stale-dist");
    let fake_trunk_dir = write_fake_trunk_bin(&format!("printf rebuilt > '{}'", marker.display()));
    let _path_guard = test_path_guard(Some(path_with_fake_trunk(&fake_trunk_dir)));

    ensure_frontend_built_at(&frontend_root, &dist)
        .await
        .expect("stale frontend dist should trigger a rebuild");

    assert_eq!(
        fs::read_to_string(&marker).expect("the fake trunk marker should be written"),
        "rebuilt"
    );
}

#[test]
fn frontend_bundle_paths_reports_non_directory_dist_errors() {
    let frontend_root = write_temp_frontend_root("dist-read-error");
    let dist = frontend_root.join("dist-file");
    fs::write(&dist, "not a directory").expect("dist file should be writable");

    let error = frontend_bundle_paths(&dist).expect_err("non-directory dist should fail");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message }
            if message.contains("reading") && message.contains("dist-file")
    ));
}

#[test]
fn latest_frontend_input_modified_reports_missing_frontend_inputs() {
    let frontend_root =
        unique_temp_json_path("acp-web-frontend-root", "empty-root").with_extension("");
    fs::create_dir_all(frontend_root.join("src"))
        .expect("empty frontend src directory should be creatable");

    let error = latest_frontend_input_modified(&frontend_root)
        .expect_err("empty frontend roots should fail");

    assert!(matches!(
        error,
        LauncherError::FrontendBuild { message }
            if message == format!("no frontend inputs were found under {}", frontend_root.display())
    ));
}
