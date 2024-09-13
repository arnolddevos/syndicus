#![feature(async_closure)]
mod log;
mod scope;

#[cfg(feature = "log")]
pub use log::{Log, Publisher, SharedSubscription, Subscription};
#[cfg(feature = "scope")]
pub use scope::{scope, simple_scope, Joiner, Tasker};
use std::hash::Hash;

/// The log element type must provide a compaction key via this trait.
pub trait Compactable
where
    Self::Key: Eq + Hash,
{
    type Key;

    /// The compaction key for this element.
    fn compaction_key(&self) -> Self::Key;
}
