use std::{future::Future, io};

use acp_app_support_errors::{ServiceReadinessError, SupportResult};

pub async fn run_service_with_readiness<E, Ready, Serve, OnReady>(
    ready: Ready,
    serve: Serve,
    on_ready: OnReady,
) -> SupportResult<(), ServiceReadinessError<E>>
where
    Ready: Future<Output = SupportResult<(), E>>,
    Serve: Future<Output = io::Result<()>>,
    OnReady: FnOnce(),
{
    tokio::pin!(ready);
    tokio::pin!(serve);

    tokio::select! {
        result = &mut ready => {
            result.map_err(ServiceReadinessError::Ready)?;
            on_ready();
            serve.await.map_err(ServiceReadinessError::Run)
        }
        result = &mut serve => result.map_err(ServiceReadinessError::Run),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use super::run_service_with_readiness;
    use acp_app_support_errors::ServiceReadinessError;

    #[tokio::test]
    async fn run_service_with_readiness_calls_the_ready_callback_before_waiting_for_shutdown() {
        let ready_called = Arc::new(AtomicBool::new(false));
        let ready_called_for_assert = ready_called.clone();

        run_service_with_readiness(
            async { Ok::<(), io::Error>(()) },
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok::<(), io::Error>(())
            },
            move || {
                ready_called.store(true, Ordering::SeqCst);
            },
        )
        .await
        .expect("service should run after readiness succeeds");

        assert!(ready_called_for_assert.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn run_service_with_readiness_surfaces_service_failures_before_ready() {
        let error = run_service_with_readiness(
            std::future::pending::<std::result::Result<(), io::Error>>(),
            std::future::ready(Err::<(), _>(io::Error::other("boom"))),
            Default::default,
        )
        .await
        .expect_err("service errors should win when they happen first");

        assert!(matches!(error, ServiceReadinessError::Run(_)));
    }

    #[tokio::test]
    async fn run_service_with_readiness_surfaces_readiness_failures() {
        let error = run_service_with_readiness(
            std::future::ready(Err::<(), _>(io::Error::other("not ready"))),
            std::future::pending::<std::result::Result<(), io::Error>>(),
            Default::default,
        )
        .await
        .expect_err("readiness failures should be surfaced");

        assert!(matches!(error, ServiceReadinessError::Ready(_)));
    }
}
