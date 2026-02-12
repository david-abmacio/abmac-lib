//! Tests for the red-blue pebble game.

use crate::game::PebbleGame;

#[test]
fn test_basic_operations() {
    let mut game: PebbleGame<u64> = PebbleGame::new(4);

    game.initialize_inputs([1, 2, 3]);
    assert_eq!(game.blue_count(), 3);
    assert_eq!(game.red_count(), 0);

    // Load from blue to red
    game.load(1).unwrap();
    assert!(game.is_red(1));
    assert!(!game.is_blue(1));
    assert_eq!(game.io_count(), 1);

    // Compute with dependency
    game.compute(4, &[1]).unwrap();
    assert!(game.is_red(4));

    // Store back to blue
    game.store(1).unwrap();
    assert!(game.is_blue(1));
    assert!(!game.is_red(1));
    assert_eq!(game.io_count(), 2);
}

#[test]
fn test_memory_limit() {
    let mut game: PebbleGame<u64> = PebbleGame::new(2);

    game.initialize_inputs([1, 2, 3]);

    game.load(1).unwrap();
    game.load(2).unwrap();

    // Should fail - red memory full
    let result = game.load(3);
    assert!(result.is_err());
}

#[test]
fn test_invariants() {
    let mut game: PebbleGame<u64> = PebbleGame::new(2);

    game.initialize_inputs([1, 2, 3]);
    game.load(1).unwrap();
    game.load(2).unwrap();

    // At capacity — invariants should still hold
    assert!(game.validate_invariants().is_ok());
    assert_eq!(game.red_count(), 2);
    assert_eq!(game.blue_count(), 1); // node 3 still blue
}

#[test]
fn test_delete() {
    let mut game: PebbleGame<u64> = PebbleGame::new(4);

    game.initialize_inputs([1]);
    game.load(1).unwrap();
    assert!(game.is_red(1));

    game.delete(1).unwrap();
    assert!(!game.is_red(1));
    assert!(!game.is_blue(1)); // Gone completely
}
