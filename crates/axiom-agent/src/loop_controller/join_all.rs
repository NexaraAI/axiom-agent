pub(super) struct SimpleJoinAll<F: std::future::Future> {
    futures: Vec<Option<F>>,
    results: Vec<Option<F::Output>>,
}

impl<F: std::future::Future> SimpleJoinAll<F> {
    pub(super) fn new(futures: Vec<F>) -> Self {
        let len = futures.len();
        Self {
            futures: futures.into_iter().map(Some).collect(),
            results: (0..len).map(|_| None).collect(),
        }
    }
}

impl<F: std::future::Future + Unpin> std::future::Future for SimpleJoinAll<F>
where
    F::Output: Unpin,
{
    type Output = Vec<F::Output>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        let mut all_done = true;
        let len = this.futures.len();
        for i in 0..len {
            if let Some(fut) = this.futures[i].as_mut() {
                match std::pin::Pin::new(fut).poll(cx) {
                    std::task::Poll::Ready(output) => {
                        this.results[i] = Some(output);
                        this.futures[i] = None;
                    }
                    std::task::Poll::Pending => {
                        all_done = false;
                    }
                }
            }
        }
        if all_done {
            let res = this
                .results
                .iter_mut()
                .map(|opt| opt.take().expect("future completed"))
                .collect();
            std::task::Poll::Ready(res)
        } else {
            std::task::Poll::Pending
        }
    }
}
