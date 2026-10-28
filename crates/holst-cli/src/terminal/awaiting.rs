//! Awaiting a future that may not be there.

pub(crate) async fn optional<F: Future>(future: Option<F>) -> Option<F::Output> {
    match future {
        Some(future) => Some(future.await),
        None => None,
    }
}

pub(crate) fn apart<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    drop(std::thread::spawn(move || sender.send(work()).ok()));
    async move { receiver.await.ok() }
}

#[cfg(test)]
mod tests {
    use crate::terminal::awaiting::{apart, optional};

    #[tokio::test]
    async fn awaits_only_what_is_there() {
        assert_eq!(optional(Some(std::future::ready(1))).await, Some(1));
        assert_eq!(optional(None::<std::future::Ready<i32>>).await, None);
    }

    #[tokio::test]
    async fn apart_gives_back_the_result_of_its_thread() {
        let main = std::thread::current().id();
        let other = apart(move || std::thread::current().id() != main).await;
        assert_eq!(other, Some(true));
    }
}
