# syndicus

A rusty interpretation of publish/subscribe using types for topics.

## Log

A `Log` is cross between a broadcast queue and a key-value store.
It is an in-process, async data structure built on tokio `watch`. 
Messages are exchanged on a many to many basis between publishers 
and subscribers.

Types are used for topics. An application defines a unified type for communication, 
typically an `enum`. Call this type `A`.  

- A publisher of messages with type `B` requires `B: Into<A>`.  
- A subscriber to messages of type `B` requires `A: TryInto<B>`.

The derive-more crate can neatly produce these conversions.

### No Blocking or Lagging

A `Log` has no backlog limit and, under certain conditions, 
will operate in bounded space. Publishers are never blocked and 
subscribers never get lagging errors. The price of this is compaction.   

### Compaction

The `Log` will drop certain older messages.
The last `linear_min` messages are always retained.  Any older message may be
dropped if it has the same `compaction_key` as a younger message.  
The order of publication of messages is preserved in any case.

> The assumption is that a subscriber only needs to see the latest message with
> each key to converge on a valid state.

### Key-Value Structure

The ability to extract a compaction key is expressed by a trait, `Compactable`.
This effectively imposes a key-value structure on the data.

The space complexity of a `Log` is O(n) where n is the number of distinct
compaction keys among the published messages. This is comparable to the
space requirement of a key-value store.
