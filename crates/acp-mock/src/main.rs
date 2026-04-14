#[tokio::main]
async fn main() -> Result<(), acp_mock::MockAppError> {
    acp_mock::run_with_args(std::env::args_os()).await
}
