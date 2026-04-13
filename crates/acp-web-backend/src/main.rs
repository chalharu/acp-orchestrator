#[tokio::main]
async fn main() -> Result<(), acp_web_backend::BackendAppError> {
    acp_web_backend::run_with_args(std::env::args_os()).await
}
