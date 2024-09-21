# syndicus

[![Crates.io Version](https://img.shields.io/crates/v/syndicus)](https://crates.io/crates/syndicus)

A rusty interpretation of publish/subscribe using types for topics
and compaction to control backlog.

## Syndicate

A `Syndicate` exchanges messages between publishers and subscribers on a many to many basis. 
It is an in-process, async data structure built on tokio `watch`. 

Types are used for topics. An application defines a unified type for communication, 
typically an `enum`. Call this type `A`.  

- A publisher of messages with type `B` requires `B: Into<A>`.  
- A subscriber to messages of type `B` requires `A: TryInto<B>`.

The [derive-more](https://crates.io/crates/derive_more) crate can neatly produce these conversions. 
Here is a toy example:

```rust
// Individual message types
#[derive(Debug, Clone)]
struct Temperature(i64);

#[derive(Debug, Clone)]
struct Voltage(i64);

// Unified message type
#[derive(Debug, Clone, From, TryInto)]
enum Message {
    T(Temperature),
    V(Voltage),
}

// The syndicate.
let syndicate: Syndicate<Message> = Default::default();
```

Then publishing and subscribing tasks for `Temperature` might look like this:

```rust
// a task that publishes temperatures
async fn temp_sensor(p: Publisher<Message, Temperature>) -> Result<()> {
    // see examples/basics/main.rs
    loop {
        // ....
        p.push(Temperature(t)).await;
    }
}

// a task that monitors temperature
async fn temp_monitor(mut s: Subscription<Message, Temperature>) -> Result<()> {
    // see examples/basics/main.rs
    while let Some(Temperature(t)) = s.pull().await {
        // ...
    }
}

// run the tasks
spawn(temp_sensor(syndicate.publish()));
spawn(temp_monitor(syndicate.subscribe()));
```

### No Blocking or Lagging

A `Syndicate` has no backlog limit meaning publishers are never blocked and 
subscribers never get lagging errors. With certain assumptions, `Syndicate`
will also operate in bounded space.  The price of this is compaction. 

### Compaction

The `Syndicate` will drop certain older messages.
The last `linear_min` messages are always retained.  Any older message may be
dropped if it has the same `compaction_key` as a younger message.
The order of publication of messages is preserved in any case.

> The assumption is that a subscriber only needs to see the latest message with
> each key to converge on a valid state.

For this example, compaction will ensure that at least the most recent message
of each type is retained.  The compaction key is just the enum `discriminant`:

```rust
impl Compactable for Message {
    type Key = Discriminant<Self>;
    fn compaction_key(&self) -> Self::Key {
        discriminant(self)
    }
}
````

### Key-Value Structure

The ability to extract a compaction key is expressed by a trait, `Compactable`.
This effectively imposes a key-value structure on the data.

The space complexity of a `Syndicate` is O(n) where n is the number of distinct
compaction keys among the published messages. This is comparable to the
space requirement of a key-value store.

## scope

Function `scope` implements a form of structured concurrency intended to
work with `Syndicate`. This is built on tokio `JoinSet`.  Tasks spawned within a scope
will be joined at the close of the scope.

Tasks are assumed to return `Result<(), Error>` where `Error` is an error type used
throughout the application.  This keeps errors and messages separate. 

For example: 

```rust
// Run tasks until all finished or any one errors
scope(|local| {
    local.spawn(temp_sensor(syndicate.publish()));
    local.spawn(volt_sensor(syndicate.publish()));
    local.spawn(temp_monitor(syndicate.subscribe()));
    Ok(())
})
.await
```
