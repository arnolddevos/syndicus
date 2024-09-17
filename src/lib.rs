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
    /// The type of the compaction key
    type Key;

    /// The compaction key for this message.
    /// In compaction messages with the same key will be merged.
    fn compaction_key(&self) -> Self::Key;

    /// Merge two messages which have the same key.
    ///
    /// The method should be associative so
    /// `a.compact(b.compact(c)) == (a.compact(b)).compact(c)`
    ///
    /// In compaction, `self` will be a younger message than the argument.
    /// The default implementation retains the younger message.
    fn compact(self, _other: Self) -> Self {
        self
    }
}
