# syndicus

![Crates.io Version](https://img.shields.io/crates/v/syndicus)

A rusty interpretation of publish/subscribe using types for topics
and compaction to control backlog.

## Syndicate

A `Syndicate` exchanges messages between publishers and subscribers on a many to many basis. 
It is an in-process, async data structure built on tokio `watch`. 

Types are used for topics. An application defines a unified type for communication, 
typically an `enum`. Call this type `A`.  

- A publisher of messages with type `B` requires `B: Into<A>`.  
- A subscriber to messages of type `B` requires `A: TryInto<B>`.

The derive-more crate can neatly produce these conversions.

### No Blocking or Lagging

A `Syndicate` has no backlog limit meaning publishers are never blocked and 
subscribers never get lagging errors. With certain assumptions, `Syndicate`
will also operate in bounded space.  The price of this is compaction. 

### Compaction

The `Syndicate` will merge, or compact, certain older messages.
The last `linear_min` messages are always retained.  
Older messages are grouped by `compaction_key` 
and each group is compacted into a single message. 

By default, just the youngest message with a given key is retained,
similar to a key-value store. 
In addtion, the order of publication is preserved. 

### Other Compaction Strategies

Messages implement the `Compactable` trait which defines the `compaction_key` method.  

It also provides a `compact` method. `a.compact(b)` consumes `b` and updates `a`.  
A group of messages is compacted as if by 
`g.reduce(|mut a, b| {a.compact(b); a})` 
where `g` is envisaged as an iterator over messages with
the same key in order from yougest to oldest.

The default implementation of `compact` does nothing.  
Alternatively, `compact` can can produce aggregations over the compacted messages.  
It may also affect the key and topic of the message.

### Space

Due to compaction, the space complexity of a `Syndicate` is O(n) 
where n is the number of distinct compaction keys among the published messages. 
This is comparable to the space requirement of a key-value store.

## scope

Function `scope` implements a form of structured concurrency intended to 
work with `Syndicate`. This is built on tokio `JoinSet`.  Tasks spawned within a scope 
will be joined at the close of the scope.  

Tasks are assumed to return `Result<(), Error>` where `Error` is an error type used 
throughout the application.  

Task input/output is assumed to be via a `Syndicate<Message>`. In this design 
`Message` and `Error` follow different paths and are handled separately.
Tasks can be managed in sets (`JoinSet`) as they all have the same result type.


