use crate::fns::FnMut1;
use crate::stream::Fuse;
use alloc::vec::Vec;
use core::mem;
use core::pin::Pin;
use futures_core::stream::Stream;
use futures_core::task::{Context, Poll};
use futures_core::{ready, Future};
use futures_sink::Sink;
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for the [`group_by`](super::StreamExt::group_by) method.
    #[must_use = "streams do nothing unless polled"]
    pub struct GroupBy<St,Fut,F>
    where
        St:Stream,
        Fut: Future,
        <Fut as Future>::Output:PartialEq,
    {
        #[pin]
        stream: Fuse<St>,
        pending_key:Option<<Fut as Future>::Output>,
        #[pin]
        pending_fut:Option<Fut>,
        pending_item:Option<St::Item>,
        items: Vec<St::Item>,
        #[pin]
        future:Option<Fut>,
        f: F,
    }
}
#[allow(single_use_lifetimes)] // https://github.com/rust-lang/rust/issues/55058
impl<St, Fut, F> GroupBy<St, Fut, F>
where
    St: Stream,
    F: for<'a> FnMut1<&'a St::Item, Output = Fut>,
    Fut: Future,
    <Fut as Future>::Output: PartialEq,
{
    pub(super) fn new(stream: St, f: F) -> Self {
        Self {
            stream: super::Fuse::new(stream),
            pending_key: None,
            items: Vec::new(),
            future: None,
            f,
            pending_fut: None,
            pending_item: None,
        }
    }
    delegate_access_inner!(stream, St, (.));
}

#[allow(single_use_lifetimes)] // https://github.com/rust-lang/rust/issues/55058
impl<St, Fut, F> Stream for GroupBy<St, Fut, F>
where
    St: Stream,
    F: for<'a> FnMut1<&'a St::Item, Output = Fut>,
    Fut: Future,
    <Fut as Future>::Output: PartialEq,
{
    type Item = (<Fut as Future>::Output, Vec<St::Item>);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        loop {
            if let Some(fut) = this.pending_fut.as_mut().as_pin_mut() {
                let res = ready!(fut.poll(cx));
                this.pending_fut.set(None);
                match this.pending_key {
                    Some(pending_key) => {
                        if pending_key == &res {
                            this.items.push(this.pending_item.take().unwrap());
                        } else {
                            let pending_key = this.pending_key.replace(res);
                            let items =
                                mem::replace(this.items, vec![this.pending_item.take().unwrap()]);
                            return Poll::Ready(Some((pending_key.unwrap(), items)));
                        }
                    }
                    None => {
                        this.items.push(this.pending_item.take().unwrap());
                        *this.pending_key = Some(res);
                    }
                }
            } else {
                match ready!(this.stream.as_mut().poll_next(cx)) {
                    Some(item) => {
                        this.pending_fut.set(Some(this.f.call_mut(&item)));
                        *this.pending_item = Some(item);
                    }
                    None => {
                        let last = if this.items.is_empty() {
                            None
                        } else {
                            let full_buf = mem::replace(this.items, Vec::new());
                            let pending_key = this.pending_key.take().unwrap();
                            Some((pending_key, full_buf))
                        };

                        return Poll::Ready(last);
                    }
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let chunk_len = if self.items.is_empty() { 0 } else { 1 };
        let (lower, upper) = self.stream.size_hint();
        let lower = lower.saturating_add(chunk_len);
        let upper = match upper {
            Some(x) => x.checked_add(chunk_len),
            None => None,
        };
        (lower, upper)
    }
}

// Forwarding impl of Sink from the underlying stream
#[cfg(feature = "sink")]
impl<S, Fut, F, Item> Sink<Item> for GroupBy<S, Fut, F>
where
    S: Stream + Sink<Item>,
    F: FnMut(&S::Item) -> Fut,
    Fut: Future,
    <Fut as Future>::Output: PartialEq,
{
    type Error = S::Error;

    delegate_sink!(stream, Item);
}
