#[tokio::main]
async fn main() -> Result<(), acp_cli::CliError> {
    acp_cli::run_with_args(std::env::args_os()).await
}
