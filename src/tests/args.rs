use super::*;

#[test]
fn split_launcher_args_defaults_to_chat_new() {
    let args = vec![OsString::from("acp")];

    assert_eq!(
        split_launcher_args(&args).expect("default launcher args should parse"),
        LauncherArgs {
            acp_server: None,
            web: false,
            cli_args: vec![OsString::from("chat"), OsString::from("--new")],
        }
    );
}

#[test]
fn split_launcher_args_preserves_explicit_arguments() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("session"),
        OsString::from("list"),
    ];

    assert_eq!(
        split_launcher_args(&args).expect("explicit launcher args should parse"),
        LauncherArgs {
            acp_server: None,
            web: false,
            cli_args: vec![OsString::from("session"), OsString::from("list")],
        }
    );
}

#[test]
fn split_launcher_args_requires_an_acp_server_value() {
    let args = vec![OsString::from("acp"), OsString::from("--acp-server")];

    let error = split_launcher_args(&args).expect_err("missing ACP server values should fail");

    assert!(matches!(error, LauncherError::MissingAcpServer));
}

#[test]
fn split_launcher_args_rejects_an_empty_acp_server_value() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("--acp-server"),
        OsString::from(""),
    ];

    let error = split_launcher_args(&args).expect_err("empty ACP server values should fail");

    assert!(matches!(error, LauncherError::MissingAcpServer));
}

#[test]
fn split_launcher_args_rejects_an_empty_equals_form_acp_server_override() {
    let args = vec![OsString::from("acp"), OsString::from("--acp-server=")];

    let error = split_launcher_args(&args).expect_err("empty ACP server values should fail");

    assert!(matches!(error, LauncherError::MissingAcpServer));
}

#[test]
fn split_launcher_args_extracts_supported_acp_server_overrides() {
    let cases = [
        vec!["acp", "--acp-server", "127.0.0.1:8090"],
        vec!["acp", "--acp-server=127.0.0.1:8090"],
        vec!["acp", "--acp-server", "127.0.0.1:8090", "chat", "--new"],
    ];

    for raw_args in cases {
        let args = raw_args.into_iter().map(OsString::from).collect::<Vec<_>>();

        assert_eq!(
            split_launcher_args(&args).expect("ACP server overrides should parse"),
            LauncherArgs {
                acp_server: Some(OsString::from("127.0.0.1:8090")),
                web: false,
                cli_args: vec![OsString::from("chat"), OsString::from("--new")],
            }
        );
    }
}

#[test]
fn split_launcher_args_extracts_web_mode_without_defaulting_to_cli_chat() {
    let args = vec![OsString::from("acp"), OsString::from("--web")];

    assert_eq!(
        split_launcher_args(&args).expect("web mode should parse"),
        LauncherArgs {
            acp_server: None,
            web: true,
            cli_args: Vec::new(),
        }
    );
}

#[test]
fn split_launcher_args_supports_web_mode_with_an_acp_server_override() {
    let args = vec![
        OsString::from("acp"),
        OsString::from("--web"),
        OsString::from("--acp-server"),
        OsString::from("127.0.0.1:8090"),
    ];

    assert_eq!(
        split_launcher_args(&args).expect("web mode with ACP overrides should parse"),
        LauncherArgs {
            acp_server: Some(OsString::from("127.0.0.1:8090")),
            web: true,
            cli_args: Vec::new(),
        }
    );
}

#[test]
fn web_backend_url_prefers_the_stack_value() {
    let stack = launcher_stack::LauncherStack::persistent(
        "https://127.0.0.1:8443".to_string(),
        "token".to_string(),
    );

    let backend_url = web_backend_url(&stack).expect("stack backend URLs should win");

    assert_eq!(backend_url, "https://127.0.0.1:8443");
}

#[test]
fn web_backend_url_falls_back_to_the_environment() {
    let _guard = lock_acp_server_url();
    let _url_guard = test_acp_server_url_guard(Some("https://127.0.0.1:9443"));

    let backend_url = web_backend_url(&launcher_stack::LauncherStack::direct())
        .expect("environment backend URLs should be used");

    assert_eq!(backend_url, "https://127.0.0.1:9443");
}

#[test]
fn web_backend_url_requires_a_value_from_the_stack_or_environment() {
    let _guard = lock_acp_server_url();
    let _url_guard = test_acp_server_url_guard(None);

    let error = web_backend_url(&launcher_stack::LauncherStack::direct())
        .expect_err("missing backend URLs should fail");

    assert!(matches!(error, LauncherError::MissingBackendUrl));
}

#[test]
fn acp_server_url_guard_restores_previous_values() {
    let _guard = lock_acp_server_url();
    let _original = test_acp_server_url_guard(Some("https://127.0.0.1:1111"));

    {
        let _restore = test_acp_server_url_guard(Some("https://127.0.0.1:2222"));
        assert_eq!(
            std::env::var("ACP_SERVER_URL").ok().as_deref(),
            Some("https://127.0.0.1:2222")
        );
    }

    assert_eq!(
        std::env::var("ACP_SERVER_URL").ok().as_deref(),
        Some("https://127.0.0.1:1111")
    );
}

#[test]
fn command_needs_backend_skips_help_and_version_only() {
    assert!(!command_needs_backend(&[OsString::from("--help")]));
    assert!(!command_needs_backend(&[OsString::from("--version")]));
    assert!(!command_needs_backend(&[
        OsString::from("chat"),
        OsString::from("--help"),
    ]));
    assert!(!command_needs_backend(&[
        OsString::from("session"),
        OsString::from("--help"),
    ]));
    assert!(!command_needs_backend(&[
        OsString::from("session"),
        OsString::from("list"),
        OsString::from("--help"),
    ]));
    assert!(command_needs_backend(&[
        OsString::from("session"),
        OsString::from("list"),
    ]));
    assert!(command_needs_backend(&[
        OsString::from("chat"),
        OsString::from("--new"),
    ]));
    assert!(command_needs_backend(&[
        OsString::from("session"),
        OsString::from("close"),
        OsString::from("s_test"),
    ]));
}

#[test]
fn cli_server_url_is_explicit_accepts_both_supported_forms() {
    assert!(cli_server_url_is_explicit(&[
        OsString::from("chat"),
        OsString::from("--server-url"),
        OsString::from("http://127.0.0.1:8080"),
    ]));
    assert!(cli_server_url_is_explicit(&[
        OsString::from("chat"),
        OsString::from("--server-url=http://127.0.0.1:8080"),
    ]));
    assert!(!cli_server_url_is_explicit(&[
        OsString::from("chat"),
        OsString::from("--new"),
    ]));
}

#[test]
fn launcher_state_path_uses_data_dir_before_home_dir() {
    let path = launcher_state_path_from(
        None,
        Some(PathBuf::from("/tmp/local-data")),
        Some(PathBuf::from("/tmp/home")),
    )
    .expect("data directory paths should resolve");

    assert_eq!(
        path,
        PathBuf::from("/tmp/local-data/acp-orchestrator/launcher-stack.json")
    );
}

#[test]
fn launcher_state_path_uses_home_dir_without_a_data_dir() {
    let path = launcher_state_path_from(None, None, Some(PathBuf::from("/tmp/home")))
        .expect("home directory paths should resolve");

    assert_eq!(
        path,
        PathBuf::from("/tmp/home/.acp-orchestrator/launcher-stack.json")
    );
}

#[test]
fn launcher_state_path_uses_explicit_override_first() {
    let path = launcher_state_path_from(
        Some(OsString::from("/tmp/acp-launcher-state.json")),
        Some(PathBuf::from("/ignored")),
        Some(PathBuf::from("/ignored-home")),
    )
    .expect("explicit launcher state paths should resolve");

    assert_eq!(path, PathBuf::from("/tmp/acp-launcher-state.json"));
}

#[test]
fn launcher_state_path_requires_a_safe_directory_without_overrides() {
    let error = launcher_state_path_from(None, None, None)
        .expect_err("missing directory hints should fail");

    assert!(matches!(
        error,
        LauncherError::MissingLauncherStateDirectory
    ));
}

#[tokio::test]
async fn run_with_args_requires_an_internal_role_name() {
    let error = run_with_args(vec![
        OsString::from("acp"),
        OsString::from("__internal-role"),
    ])
    .await
    .expect_err("missing internal role should fail");

    assert!(matches!(error, LauncherError::MissingInternalRole));
}
