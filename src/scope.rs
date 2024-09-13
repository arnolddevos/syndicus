#![cfg(feature = "scope")]

use std::{future::Future, sync::Arc};
use tokio::{
    sync::Mutex,
    task::{AbortHandle, JoinSet},
};

/// An opinionated wrapper around tokio `JoinSet`.
/// Tasks are assumed to return a `Result<(),E>` and
/// the `join_all` function aborts all remaining tasks if
/// any one produces an error.
///
/// While a `Joiner` provides join functions, task spawning is
/// handled by a `Tasker`.
///
/// Unlike JoinSet, this is Send + Sync.  (It is not Clone because
/// it is not intended share the underlying JoinSet across tasks, for now.)
///
/// The `JoinSet` is managed by a mutex so that `Tasker` can be
/// passed into static async blocks.   
#[derive(Debug)]
pub struct Joiner<E>(Arc<Mutex<JoinSet<Result<(), E>>>>);

impl<E> Joiner<E>
where
    E: 'static,
{
    /// Create a Joiner
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(JoinSet::new())))
    }

    /// Create a Tasker that can spawn tasks onto the wrapped `JoinSet`
    pub fn tasker(&self) -> Tasker<E> {
        Tasker(self.0.clone())
    }

    /// Join the next completed or aborted task
    pub async fn join_next(&self) -> Option<Result<(), E>> {
        let mut set = self.0.lock().await;
        let next = set.join_next().await;
        next.map(|outer| match outer {
            Ok(inner) => inner, // task completion
            Err(_) => Ok(()),   // task was (deliberately?) aborted
        })
    }

    /// Join all tasks in the `JoinSet` as they complete.
    /// If a task completes with an error, abort all remaining tasks.
    pub async fn join_all(self) -> Result<(), E> {
        loop {
            match self.join_next().await {
                Some(Ok(())) => (), // a task succeeded or aborted
                Some(e) => {
                    // a task returned error
                    self.shutdown().await;
                    break e;
                }
                None => break Ok(()), // all tasks succeeded or aborted
            }
        }
    }

    /// Abort all tasks in the `JoinSet`
    pub async fn shutdown(self) {
        let mut set = self.0.lock().await;
        set.shutdown().await
    }
}

/// A wrapper around `JoinSet` that providing just the ability to spawn tasks.
/// Unlike JoinSet, this is Send + Sync.  (It is not Clone because it is not
/// intended share the underlying JoinSet across tasks, for now.)
#[derive(Debug)]
pub struct Tasker<E>(Arc<Mutex<JoinSet<Result<(), E>>>>);

impl<E> Tasker<E>
where
    E: Send + 'static,
{
    pub async fn spawn<F>(&self, body: F) -> AbortHandle
    where
        F: Future<Output = Result<(), E>> + 'static + Send,
    {
        let mut t = self.0.lock().await;
        t.spawn(body)
    }

    pub async fn spawn_blocking<F>(&self, body: F) -> AbortHandle
    where
        F: FnOnce() -> Result<(), E> + 'static + Send,
    {
        let mut t = self.0.lock().await;
        t.spawn_blocking(body)
    }
}

/// Run an async function passing a `Tasker` and then join all spawned tasks.
/// Return the result of the function if it and its tasks succeed.  
/// Otherwise return first error encountered.
/// It is not intended that the `Tasker` escape the function or
/// the future it returns although the types do not prevent this.
pub async fn scope<F, D, A, E>(body: F) -> Result<A, E>
where
    F: FnOnce(Tasker<E>) -> D,
    D: Future<Output = Result<A, E>>,
    E: 'static + Send,
{
    let joiner = Joiner::new();
    let tasker = joiner.tasker();
    match body(tasker).await {
        Ok(a) => match joiner.join_all().await {
            Ok(()) => Ok(a),
            Err(e) => Err(e),
        },
        Err(e) => {
            joiner.shutdown().await;
            Err(e)
        }
    }
}

pub async fn simple_scope<A, E>(
    body: impl async FnOnce(&mut JoinSet<Result<(), E>>) -> Result<A, E>,
) -> Result<A, E>
where
    E: 'static,
{
    let mut set = JoinSet::<Result<(), E>>::new();

    // Join the next completed or aborted task
    let join_next = async |set: &mut JoinSet<Result<(), E>>| {
        let next = set.join_next().await;
        next.map(|outer| match outer {
            Ok(inner) => inner, // task completion
            Err(_) => Ok(()),   // task was (deliberately?) aborted
        })
    };

    // Join all tasks in the `JoinSet` as they complete.
    // If a task completes with an error, abort all remaining tasks.
    let join_all = async |set: &mut JoinSet<Result<(), E>>| {
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
    };

    let result = body(&mut set).await;
    if result.is_ok() {
        join_all(&mut set).await?
    } else {
        set.shutdown().await;
    }
    result
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_scopes() {
        let task_load = 100;
        let counter = scope(|tasker: Tasker<()>| async move {
            let counter = Arc::new(Mutex::new(0usize));
            for _i in 0..task_load {
                let c = counter.clone();
                tasker
                    .spawn(async move {
                        sleep(Duration::from_millis(100)).await;
                        *c.lock().await += 1;
                        Ok(())
                    })
                    .await;
            }
            Ok(counter)
        })
        .await
        .unwrap();
        assert!(*counter.lock().await == task_load)
    }

    #[tokio::test]
    async fn test_simple_scope() {
        let task_load = 100;
        let counter = simple_scope::<_, ()>(async |tasker| {
            let counter = Arc::new(Mutex::new(0usize));
            for _i in 0..task_load {
                let c = counter.clone();
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
