async fn run_with_args<I, T>(args: I) -> Result<(), acp_cli::CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    acp_cli::run_with_args(args).await
}

#[tokio::main]
async fn main() -> Result<(), acp_cli::CliError> {
    run_with_args(std::env::args_os()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_with_args_delegates_to_the_cli_library() {
        let error = run_with_args(["acp", "session", "list"])
            .await
            .expect_err("delegated session list should surface cli errors");
        assert!(matches!(
            error,
            acp_cli::CliError::MissingServerUrl { command } if command == "listing sessions"
        ));
    }
}
