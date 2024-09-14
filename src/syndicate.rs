#![cfg(feature = "log")]

use crate::Compactable;
use std::{collections::HashSet, marker::PhantomData, sync::Arc};
use tokio::{
    sync::{
        watch::{Receiver, Sender},
        Mutex,
    },
    task::yield_now,
};

/// A `Syndicate` is a rusty interpretation of publish/subscribe using types for topics.
/// It is an in-process, async data structure built on tokio `watch`.
///
/// An application defines a unified type for communication, typically
/// an `enum`. Call this type `A`.  Then:
///
/// - A publisher of messages with type `B` requires `B: Into<A>`.  
/// - A subscriber to messages of type `B` requires `A: TryInto<B>`.
///
/// The derive-more crate can neatly produce these conversions.
///
/// Unlike the various async queues available in rust,
/// `Syndicate` takes bounded space _and_ has no backlog limit.
///
/// Publishers are never blocked, although they may be
/// made to yield to exert light back-pressure.  
///
/// Subscribers never get lagging errors and can join at any
/// time and catch up to the current state.
///
/// The price of this is compaction.   The `Syndicate` will drop certain older messages.
/// The last `linear_min` messages are always retained.  Any older message may be
/// dropped if it has the same `compaction_key` as a younger message.  
/// The order of publication of messages is preserved in any case.
///
/// The ability to extract a compaction key is expressed by a trait, `Compactable`.
/// This effectively imposes a key-value structure on the data.
///
/// The space complexity of a `Syndicate` is O(n) where n is the number of distinct
/// compaction keys among the published messages. This is comparable to the
/// space requirement of a key-value store.
#[derive(Debug, Clone)]
pub struct Syndicate<A> {
    sender: Sender<Inner<A>>,
}

impl<A> Syndicate<A> {
    /// Create a new syndicate that retains the last  `linear_min` messages and
    /// compacts all messages older than the last `linear_max` massages.
    /// Producers will yield when there are linear_hi retained messages.
    pub fn new(linear_min: usize, linear_hi: usize, linear_max: usize) -> Self {
        let sender = Sender::new(Inner::<A> {
            linear: Vec::new(),
            non_linear: Vec::new(),
            offset: 0,
            linear_max,
            linear_hi,
            linear_min,
        });
        Self { sender }
    }

    /// Create a new `Subscription` for messages later than a given offset.
    ///
    /// If the offset is `0` all messages of type `B` are subscribed.
    /// If an offset returned by `Syndicate::snapshot` is given, all messages
    /// of type `B` following the last message in the snapshot are subscribed.
    ///
    /// Note: offset values increase monotonically but are not sequential.  
    pub fn subscribe_at<B>(&self, offset: usize) -> Subscription<A, B>
    where
        A: Clone,
    {
        let mut receiver = self.sender.subscribe();
        let (offset, backlog) = receiver.borrow_and_update().since(offset);
        Subscription {
            backlog,
            offset,
            receiver,
            marker: PhantomData,
        }
    }

    /// Create a new subscription to messages of type `B`.
    /// Messages are return in order from the start of
    /// the syndicate onwards.  Messages removed by compaction
    /// are omitted.
    pub fn subscribe<B>(&self) -> Subscription<A, B>
    where
        A: Clone,
    {
        self.subscribe_at(0)
    }

    // Create a new `Publisher`.
    pub fn publish<B>(&self) -> Publisher<A, B> {
        Publisher {
            sender: self.sender.clone(),
            marker: PhantomData,
        }
    }

    /// The entire contents of the syndicate at this moment.
    pub fn snapshot(&self, offset: usize) -> (usize, Vec<A>)
    where
        A: Clone,
    {
        let (offset, mut elements) = self.sender.borrow().since(offset);
        elements.reverse();
        (offset, elements)
    }
}

/// The data structure of a `Syndicate` which is manged by a tokio `watch`.
#[derive(Debug)]
struct Inner<A> {
    /// the most recent elements with contiguous ascending offsets.
    linear: Vec<A>,
    /// older elements with with non-contiguous descending offsets  
    non_linear: Vec<Indexed<A>>,
    /// the greatest offset seen or zero if no elements have been seen
    offset: usize,
    /// compaction trigger length for the linear vector
    linear_max: usize,
    /// yield trigger length for the linear vector
    linear_hi: usize,
    /// compaction target length for the linear vector
    linear_min: usize,
}

impl<A> Inner<A>
where
    A: Clone,
{
    /// every element with offset > given offset in order youngest to oldest and the highest offset therein
    fn since(&self, offset: usize) -> (usize, Vec<A>) {
        // do we have the requested offset?
        if offset < self.offset {
            let offset0 = self.offset - self.linear.len(); // offset before oldest linear element

            // satisfy from the linear log?
            let elements = if offset >= offset0 {
                let bound = offset - offset0; // how many elements to skip
                self.linear[bound..].iter().rev().cloned().collect()
            } else {
                // filter values from the non-linear log
                let non_linear = self
                    .non_linear
                    .iter()
                    .take_while(|c| c.offset > offset)
                    .map(|c| &c.value);

                // prepend the whole linear log
                self.linear
                    .iter()
                    .rev()
                    .chain(non_linear)
                    .cloned()
                    .collect()
            };
            (self.offset, elements)
        } else {
            (offset, Vec::new())
        }
    }
}

impl<A> Inner<A>
where
    A: Compactable,
{
    /// Push a new element onto the log. Returns true if the calling is advised to yield.
    fn push(&mut self, value: A) -> bool {
        self.linear.push(value);
        self.offset += 1;

        // is the linear log getting too long?
        if self.linear.len() >= self.linear_max {
            self.compact()
        }

        // is the linear log somewhat long?
        self.linear.len() >= self.linear_hi
    }

    /// Remove messages from the linear part of the log, preserving the most
    /// recent `linear_min`.  Add the messages to the nonlinear log and compact it.
    fn compact(&mut self) {
        // is the linear part of the log longer than minimum?
        if self.linear.len() > self.linear_min {
            let bound = self.linear.len() - self.linear_min; // how much to compact
            let offset0 = self.offset + 1 - self.linear.len(); // offset of oldest linear element

            // keep youngest part of the linear log
            let retained = self.linear[bound..].iter();

            // record compaction keys in retained linear log
            let mut keys: HashSet<A::Key> = retained.map(|a| a.compaction_key()).collect();

            // remove oldest part of the linear log
            let removed = self.linear.drain(0..bound);

            // convert to compacted format and ordering
            let new_compact = removed
                .enumerate()
                .map(|(i, a)| Indexed {
                    offset: i + offset0,
                    value: a,
                })
                .rev();

            // previously compacted elements
            let old_compact = self.non_linear.drain(..);

            // filter and collect the compacted log, from youngest to oldest elements
            let compact: Vec<Indexed<A>> = new_compact
                .chain(old_compact)
                .filter(|c| keys.insert(c.value.compaction_key()))
                .collect();

            self.linear
                .reserve_exact(self.linear_max - self.linear.len());
            self.non_linear = compact;
        }
    }
}

/// A handle to the syndicate that returns the published messages in order.
/// It carries an offset into the syndicate and caches the next values to be
/// pulled via this subscription.  
///
/// The subscription converts and filters the syndicate messages from
/// type `A` to `B` via `TryInto`.
///
/// It is not designed to be cloned. The principal method, `pull`
/// required an exclusize reference. See `SharedSubscription` for a
/// handle that can be shared between tasks.
#[derive(Debug)]
pub struct Subscription<A, B> {
    offset: usize,
    backlog: Vec<A>,
    receiver: Receiver<Inner<A>>,
    marker: PhantomData<B>,
}

impl<A, B> Subscription<A, B>
where
    A: Clone + TryInto<B>,
{
    /// Get the next message or None if there are no more publishers.
    pub async fn pull(&mut self) -> Option<B> {
        loop {
            if let Some(value) = self.backlog.pop() {
                if let Ok(value) = value.try_into() {
                    break Some(value);
                }
            } else {
                if self.receiver.changed().await.is_ok() {
                    (self.offset, self.backlog) =
                        self.receiver.borrow_and_update().since(self.offset)
                } else {
                    break None;
                }
            }
        }
    }

    /// Convert to a `SharedSubscription`
    pub fn share(self) -> SharedSubscription<A, B> {
        SharedSubscription {
            shared: Arc::new(Mutex::new(self)),
        }
    }
}

/// A handle to the syndicate that returns the published messages in order.
/// It carries an offset into the syndicate and caches the next values to be
/// pulled via this subscription.  
///
/// The subscription converts and filters the syndicate messages from
/// type `A` to `B` via `TryInto`.
///
/// A `SharedSubscription` can be cloned and a given message will be delivered
/// by at most one of the clones.  This is useful to distribute messages
/// among tasks.
#[derive(Debug, Clone)]
pub struct SharedSubscription<A, B> {
    shared: Arc<Mutex<Subscription<A, B>>>,
}

impl<A, B> SharedSubscription<A, B>
where
    A: Clone + TryInto<B>,
{
    /// Get the next message or None if there are no more publishers.
    pub async fn pull(&self) -> Option<B> {
        self.shared.lock().await.pull().await
    }
}

/// A handle to the syndicate through which new messages can be published.
///
/// The publisher accepts messages of type `B` and converts them to
/// syndicate messages of type `A` via `Into`.
#[derive(Debug, Clone)]
pub struct Publisher<A, B> {
    sender: Sender<Inner<A>>,
    marker: PhantomData<B>,
}

impl<A, B> Publisher<A, B>
where
    A: Compactable,
    B: Into<A>,
{
    /// Put a new message on the syndicate.
    pub async fn push(&self, value: B) {
        let mut heavy = false;
        self.sender
            .send_modify(|inner| heavy = inner.push(value.into()));
        if heavy {
            yield_now().await;
        }
    }
}

/// A log element and its offset (used in the non linear part of the log).
#[derive(Debug)]
struct Indexed<A> {
    offset: usize,
    value: A,
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{scope, Compactable};
    use rand::Rng;
    use std::iter::repeat_with;
    use tokio::task::JoinSet;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Message(usize, usize);

    impl Compactable for Message {
        type Key = usize;
        fn compaction_key(&self) -> Self::Key {
            self.0
        }
    }

    #[tokio::test]
    async fn test_interleaved() {
        let (_, p, mut s, test_data) = fixtures();
        let run_length = test_data.len();

        scope(async |tasker: &mut JoinSet<Result<(), String>>| {
            tasker.spawn(async move {
                fill_log(p, test_data).await;
                Ok(())
            });

            tasker.spawn(async move {
                let mut count = 0;
                let mut prev = 0;
                while let Some(Message(_, j)) = s.pull().await {
                    count += 1;
                    if j > prev {
                        prev = j
                    } else {
                        return Err(format!("Messages out of order {prev}, {j}"));
                    }
                }
                if count == run_length {
                    Ok(()) // for interleaved operation we see every message (no compaction)
                } else {
                    Err(format!("Messages received/sent = {}/{}", count, run_length))
                }
            });

            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_compaction() {
        let (_, p, mut s, test_data) = fixtures();
        let run_length = test_data.len();

        fill_log(p, test_data).await;

        let mut count = 0;
        let mut prev = 0;
        while let Some(Message(_, j)) = s.pull().await {
            count += 1;
            assert!(j > prev);
            prev = j;
        }

        // for a long enough run we will see every possible key at least once
        // but not more than the max length of the non-linear + linear logs
        assert!(
            count >= 15 && count < 35,
            "Messages received/sent = {}/{} expected 15..35",
            count,
            run_length
        );
    }

    #[tokio::test]
    async fn test_snapshot() {
        let (l, p, mut s, test_data) = fixtures();

        fill_log(p, test_data).await;

        let (_, results) = l.snapshot(0);

        for m in results {
            assert_eq!(m, s.pull().await.unwrap())
        }
    }

    #[tokio::test]
    async fn test_subscribe_at() {
        let (l, p1, _, mut test_data_a) = fixtures();
        let p2 = p1.clone();
        let test_data_b = test_data_a.split_off(test_data_a.len() / 2);

        fill_log(p1, test_data_a).await;

        let (offset, _) = l.snapshot(0);

        let mut s = l.subscribe_at::<Message>(offset);

        fill_log(p2, test_data_b).await;

        let (_, results_b) = l.snapshot(offset);

        for m in results_b {
            assert_eq!(m, s.pull().await.unwrap())
        }
    }

    fn fixtures() -> (
        Syndicate<Message>,
        Publisher<Message, Message>,
        Subscription<Message, Message>,
        Vec<Message>,
    ) {
        let log = empty_log();
        let p = log.publish();
        let s = log.subscribe();
        (log, p, s, data())
    }

    fn empty_log() -> Syndicate<Message> {
        let linear_min = 10;
        let linear_hi = 15;
        let linear_max = 20;
        Syndicate::new(linear_min, linear_hi, linear_max)
    }

    fn data() -> Vec<Message> {
        let key_space = 15;
        let run_length = 1007;
        let arb = repeat_with(|| rand::thread_rng().gen_range(0usize..key_space));
        let seq = (1usize..).into_iter();
        arb.zip(seq)
            .map(|(i, j)| Message(i, j))
            .take(run_length)
            .collect()
    }

    async fn fill_log(p: Publisher<Message, Message>, test_data: Vec<Message>) {
        for m in test_data {
            p.push(m).await;
        }
    }
}
