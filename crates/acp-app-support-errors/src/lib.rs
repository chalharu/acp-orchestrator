use std::{error::Error as StdError, io};

pub type BoxError = Box<dyn StdError + Send + Sync>;
pub type SupportResult<T, E> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum ServiceReadinessError<E> {
    Ready(E),
    Run(io::Error),
}

#[derive(Debug, snafu::Snafu)]
pub enum ListenerSetupError {
    #[snafu(display("binding the {service_name} on {host}:{port} failed"))]
    Bind {
        source: io::Error,
        service_name: &'static str,
        host: String,
        port: u16,
    },

    #[snafu(display("reading the bound {service_name} address failed"))]
    ReadBoundAddress {
        source: io::Error,
        service_name: &'static str,
    },
}
