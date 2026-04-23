#![cfg_attr(not(target_family = "wasm"), allow(unused_imports))]

//! Thin async wrappers over the ACP backend REST/SSE API.

mod accounts;
mod auth;
mod errors;
mod paths;
mod request;
#[cfg(target_family = "wasm")]
mod response;
mod sessions;
mod stream;

pub(crate) use accounts::{create_account, delete_account, list_accounts, update_account};
pub(crate) use auth::{auth_status, bootstrap_register, sign_in, sign_out};
pub(crate) use errors::SessionLoadError;
pub(crate) use paths::{decode_component, encode_component, permission_url, session_path};
#[cfg(target_family = "wasm")]
pub(crate) use request::{csrf_token, patch_json_with_csrf, post_json_with_csrf};
#[cfg(target_family = "wasm")]
pub(crate) use response::{classify_session_load_failure, response_error_message};
pub(crate) use sessions::{
    cancel_turn, create_session, delete_session, list_sessions, load_session, rename_session,
    resolve_permission, send_message,
};
pub(crate) use stream::{SseItem, open_session_event_stream};

#[cfg(test)]
pub(crate) fn poll_ready<T>(future: impl std::future::Future<Output = T>) -> T {
    struct NoopWaker;

    impl std::task::Wake for NoopWaker {
        fn wake(self: std::sync::Arc<Self>) {}
    }

    let waker = std::task::Waker::from(std::sync::Arc::new(NoopWaker));
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(&waker);
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => panic!("future unexpectedly pending in host-side API test"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::{Future, ready},
        pin::Pin,
        task::{Context, Poll},
    };

    use super::poll_ready;

    struct WakeThenPending;

    impl Future for WakeThenPending {
        type Output = ();

        fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }

    #[test]
    fn poll_ready_returns_outputs_from_ready_futures() {
        assert_eq!(poll_ready(ready("done")), "done");
    }

    #[test]
    fn poll_ready_panics_when_future_is_pending() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            poll_ready(WakeThenPending);
        }));

        assert!(result.is_err());
    }
}
