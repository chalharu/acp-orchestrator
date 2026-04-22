use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing() {
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .without_time()
                .with_writer(std::io::stderr),
        )
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::init_tracing;

    #[test]
    fn init_tracing_can_be_called_more_than_once() {
        init_tracing();
        init_tracing();
    }
}
