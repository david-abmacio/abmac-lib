//! Basic usage of the pebble checkpoint manager.
//!
//! Run with: cargo run -p pebble --features derive,serde --example basic

use pebble::{Checkpoint, DirectStorage, NoWarm, PebbleBuilder};
use serde::{Deserialize, Serialize};
use spout::DropSpout;

#[derive(Clone, Debug, Checkpoint, Serialize, Deserialize)]
struct GameState {
    #[checkpoint(id)]
    turn: u64,
    score: u32,
    lives: u8,
}

fn main() {
    let mut manager = PebbleBuilder::new()
        .cold(DirectStorage::debug())
        .warm(NoWarm)
        .log(DropSpout)
        .hot_capacity(4)
        .build::<GameState>();

    println!("Adding 10 checkpoints (hot capacity = 4):\n");

    for i in 1..=10 {
        manager
            .add(
                GameState {
                    turn: i,
                    score: (i * 100) as u32,
                    lives: 3,
                },
                &[],
            )
            .unwrap();

        let hot: Vec<u64> = (1..=i).filter(|&id| manager.is_hot(id)).collect();
        let cold: Vec<u64> = (1..=i).filter(|&id| manager.is_in_storage(id)).collect();
        println!("  turn {i:>2} | hot: {hot:?} | cold: {cold:?}");
    }

    println!("\nLoading turn 3 from cold:\n");

    let cp = manager.load(3).unwrap().clone();

    let hot: Vec<u64> = (1..=10).filter(|&id| manager.is_hot(id)).collect();
    let cold: Vec<u64> = (1..=10).filter(|&id| manager.is_in_storage(id)).collect();
    println!("  hot: {hot:?} | cold: {cold:?}");

    println!("\n{}", serde_json::to_string_pretty(&cp).unwrap());
}
