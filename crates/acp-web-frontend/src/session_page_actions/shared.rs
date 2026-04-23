#![cfg_attr(not(target_family = "wasm"), allow(dead_code))]

use core::future::Future;

#[cfg(target_family = "wasm")]
pub(super) fn spawn_browser_task(task: impl Future<Output = ()> + 'static) {
    leptos::task::spawn_local(task);
}

#[cfg(not(target_family = "wasm"))]
pub(super) fn spawn_browser_task<Task>(_task: Task)
where
    Task: Future<Output = ()> + 'static,
{
}

#[cfg(test)]
mod tests {
    use super::spawn_browser_task;

    #[test]
    fn host_spawn_browser_task_accepts_futures_without_panicking() {
        spawn_browser_task(async {});
    }
}
