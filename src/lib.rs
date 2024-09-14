mod scope;
mod syndicate;

#[cfg(feature = "scope")]
pub use scope::scope;
#[cfg(feature = "log")]
pub use syndicate::{Publisher, SharedSubscription, Subscription, Syndicate};

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
