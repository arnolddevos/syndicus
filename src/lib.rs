#![doc=include_str!("../README.md")]
mod scope;
mod syndicate;
#[cfg(feature = "scope")]
pub use scope::scope;
#[cfg(feature = "log")]
pub use syndicate::{Publisher, SharedSubscription, Subscription, Syndicate};

use std::hash::Hash;

/// The message type of a `Syndicate` must provide a compaction key via this trait.
pub trait Compactable
where
    Self: Sized,
    Self::Key: Eq + Hash,
{
    type Key;

    /// The compaction key for this element.
    fn compaction_key(&self) -> Self::Key;

    /// combine two elements
    fn compact(self, _other: Self) -> Self {
        self
    }
}
