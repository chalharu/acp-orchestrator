use tokio::net::TcpListener;

use acp_app_support_errors::ListenerSetupError;
use acp_app_support_tracing::init_tracing;

pub async fn bind_listener(
    host: &str,
    port: u16,
    service_name: &'static str,
) -> Result<TcpListener, ListenerSetupError> {
    init_tracing();

    TcpListener::bind((host, port))
        .await
        .map_err(|source| ListenerSetupError::Bind {
            source,
            service_name,
            host: host.to_string(),
            port,
        })
}

pub fn listener_endpoint(
    listener: &TcpListener,
    service_name: &'static str,
    startup_prefix: &'static str,
) -> Result<String, ListenerSetupError> {
    let address = listener
        .local_addr()
        .map_err(|source| ListenerSetupError::ReadBoundAddress {
            source,
            service_name,
        })?;
    Ok(format!("{startup_prefix}{address}"))
}

pub fn print_startup_line(startup_label: &'static str, endpoint: &str) {
    println!("{startup_label} listening on {endpoint}");
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::{bind_listener, listener_endpoint};
    use acp_app_support_errors::ListenerSetupError;

    #[tokio::test]
    async fn bind_listener_reports_successful_binding() {
        let listener = bind_listener("127.0.0.1", 0, "test service")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose its address");

        assert!(address.port() > 0);
    }

    #[tokio::test]
    async fn bind_listener_reports_bind_failures() {
        let occupied = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let port = occupied
            .local_addr()
            .expect("listener should expose its address")
            .port();

        let error = bind_listener("127.0.0.1", port, "test service")
            .await
            .expect_err("occupied ports should fail");

        assert!(
            matches!(error, ListenerSetupError::Bind { port: bound_port, .. } if bound_port == port)
        );
    }

    #[tokio::test]
    async fn listener_endpoint_formats_the_bound_address() {
        let listener = bind_listener("127.0.0.1", 0, "test service")
            .await
            .expect("listener should bind");

        let endpoint = listener_endpoint(&listener, "test service", "http://")
            .expect("endpoint should format");

        assert!(endpoint.starts_with("http://127.0.0.1:"));
    }
}
