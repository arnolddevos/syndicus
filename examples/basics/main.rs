#![allow(dead_code)]
use std::{
    mem::{discriminant, Discriminant},
    result,
    time::Duration,
};

use derive_more::{From, TryInto};
use rand::{Rng, SeedableRng};
use syndicus::{scope, Compactable, Publisher, Subscription, Syndicate};
use tokio::time::sleep;

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

// Using the discriminant as a key means we
// retain at least one of each message type
// after compaction.
impl Compactable for Message {
    type Key = Discriminant<Self>;
    fn compaction_key(&self) -> Self::Key {
        discriminant(self)
    }
}

// Application error type
#[derive(Debug)]
enum Error {
    VoltageSensorFailed,
}
type Result<A> = result::Result<A, Error>;

#[tokio::main]
async fn main() -> Result<()> {
    // a task that publishes temperatures
    async fn temp_sensor(p: Publisher<Message, Temperature>) -> Result<()> {
        let mut rng = rand::rngs::StdRng::from_entropy();
        loop {
            let t = rng.gen_range(0..50);
            p.push(Temperature(t)).await;
            sleep(Duration::from_millis(90)).await
        }
    }

    // a task that publishes voltages but fails after a while
    async fn volt_sensor(p: Publisher<Message, Voltage>) -> Result<()> {
        let mut rng = rand::rngs::StdRng::from_entropy();
        for _ in 0..100 {
            let v = rng.gen_range(0..500);
            p.push(Voltage(v)).await;
            sleep(Duration::from_millis(100)).await
        }
        Err(Error::VoltageSensorFailed)
    }

    // a task that monitors temperature
    async fn temp_monitor(mut s: Subscription<Message, Temperature>) -> Result<()> {
        if let Some(Temperature(mut t1)) = s.pull().await {
            while let Some(Temperature(t2)) = s.pull().await {
                if t1 > 30 && t2 > 30 {
                    println!("Alert! High temp {}", t1.max(t2))
                }
                t1 = t2;
            }
        }
        Ok(())
    }

    // Create a syndicate
    let syndicate = Syndicate::<Message>::new(10, 20, 25);

    // Run tasks until all finished or any one errors
    scope(|local| {
        local.spawn(temp_sensor(syndicate.publish()));
        local.spawn(volt_sensor(syndicate.publish()));
        local.spawn(temp_monitor(syndicate.subscribe()));
        Ok(())
    })
    .await
}
