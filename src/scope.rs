#![cfg(feature = "scope")]

use tokio::task::JoinSet;

/// Run a function, the scope body, passing in a `JoinSet`. Join all spawned tasks.
/// Return the result of the body if it and its tasks succeed.  
/// Otherwise return first error encountered.
pub async fn scope<A, E>(
    body: impl FnOnce(&mut JoinSet<Result<(), E>>) -> Result<A, E>,
) -> Result<A, E>
where
    E: 'static,
{
    let mut set = JoinSet::<Result<(), E>>::new();

    let result = body(&mut set);
    if result.is_ok() {
        join_all(&mut set).await?;
    } else {
        set.shutdown().await;
    }
    result
}

/// Join the next completed or aborted task
async fn join_next<E>(set: &mut JoinSet<Result<(), E>>) -> Option<Result<(), E>>
where
    E: 'static,
{
    let next = set.join_next().await;
    next.map(|outer| match outer {
        Ok(inner) => inner, // task completion, possibly with error
        Err(_) => Ok(()),   // task was (deliberately?) aborted
    })
}

/// Join all tasks in the `JoinSet` as they complete.
/// If a task completes with an error, abort all remaining tasks.
async fn join_all<E>(set: &mut JoinSet<Result<(), E>>) -> Result<(), E>
where
    E: 'static,
{
    loop {
        match join_next(set).await {
            Some(Ok(())) => (), // a task succeeded or aborted
            Some(e) => {
                // a task returned error
                set.shutdown().await;
                break e;
            }
            None => break Ok(()), // all tasks succeeded or aborted
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::{sync::Arc, time::Duration};
    use tokio::{sync::Mutex, time::sleep};

    #[tokio::test]
    async fn test_simple_scope() {
        let task_load = 100;
        let counter = scope::<_, ()>(|tasker| {
            let counter = Arc::new(Mutex::new(0usize));
            for _i in 0..task_load {
                let c = counter.clone(); // to each task a counter
                tasker.spawn(async move {
                    sleep(Duration::from_millis(100)).await;
                    *c.lock().await += 1;
                    Ok(())
                });
            }
            Ok(counter)
        })
        .await
        .unwrap();
        assert!(*counter.lock().await == task_load)
    }
}
