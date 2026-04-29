#[tokio::main]
async fn main() -> Result<(), acp_web::BackendAppError> {
    acp_web::run_with_args(std::env::args_os()).await
}
