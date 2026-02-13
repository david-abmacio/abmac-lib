//! Standalone Red-Blue Pebble Game simulator (Hong & Kung, STOC 1981).
//!
//! This module provides a rule-enforcing simulator for validating pebbling
//! strategies independently of [`PebbleManager`](crate::PebbleManager).
//! It is not used by the checkpoint manager itself.
//!
//! Rules: R1 Load (blue->red), R2 Store (red->blue), R3 Compute, R4 Delete.

use alloc::vec::Vec;
use core::fmt;
use core::hash::Hash;
use hashbrown::HashSet;

/// Memory hierarchy: Red (fast) or Blue (slow/serialized).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PebbleColor {
    Red,
    Blue,
}

/// Pebble game operations.
#[derive(Debug, Clone)]
pub enum PebbleOperation<T> {
    Load(T),
    Store(T),
    Compute(T),
    Delete(T),
}

pub use crate::errors::game::PebbleError;

pub(crate) type Result<T> = core::result::Result<T, PebbleError>;

/// Pebble game rules enforcement.
pub struct PebbleRules;

impl PebbleRules {
    pub fn can_load<T: Copy + Eq + Hash>(
        node: T,
        blue_pebbles: &HashSet<T>,
        red_pebbles: &HashSet<T>,
        max_red_pebbles: usize,
    ) -> bool {
        blue_pebbles.contains(&node)
            && !red_pebbles.contains(&node)
            && red_pebbles.len() < max_red_pebbles
    }

    pub fn can_store<T: Copy + Eq + Hash>(node: T, red_pebbles: &HashSet<T>) -> bool {
        red_pebbles.contains(&node)
    }

    pub fn can_compute<T: Copy + Eq + Hash>(
        node: T,
        dependencies: &[T],
        red_pebbles: &HashSet<T>,
        max_red_pebbles: usize,
    ) -> bool {
        let deps_ok = dependencies.iter().all(|dep| red_pebbles.contains(dep));
        let space_ok = red_pebbles.contains(&node) || red_pebbles.len() < max_red_pebbles;
        deps_ok && space_ok
    }

    pub fn can_delete<T: Copy + Eq + Hash>(node: T, red_pebbles: &HashSet<T>) -> bool {
        red_pebbles.contains(&node)
    }
}

/// Pebble game state tracker. Call `clear_log()` periodically to prevent unbounded growth.
#[must_use]
pub struct PebbleGame<T: Copy + Eq + Hash> {
    red_pebbles: HashSet<T>,
    blue_pebbles: HashSet<T>,
    max_red_pebbles: usize,
    io_count: usize,
    operation_log: Vec<PebbleOperation<T>>,
}

impl<T: Copy + Eq + Hash + fmt::Debug> PebbleGame<T> {
    pub fn new(max_red_pebbles: usize) -> Self {
        Self {
            red_pebbles: HashSet::new(),
            blue_pebbles: HashSet::new(),
            max_red_pebbles,
            io_count: 0,
            operation_log: Vec::new(),
        }
    }

    pub fn initialize_inputs(&mut self, inputs: impl IntoIterator<Item = T>) {
        for input in inputs {
            self.blue_pebbles.insert(input);
        }
    }

    /// R1: Load from blue to red (1 I/O).
    ///
    /// **Move semantics**: the blue pebble is removed and a red pebble
    /// placed. The classic Hong & Kung formulation copies (blue stays),
    /// but this implementation uses move semantics to match the
    /// `PebbleManager` where a loaded checkpoint occupies hot memory
    /// and its cold-tier slot is not freed.
    pub fn load(&mut self, node: T) -> Result<()> {
        if !PebbleRules::can_load(
            node,
            &self.blue_pebbles,
            &self.red_pebbles,
            self.max_red_pebbles,
        ) {
            return Err(PebbleError::InvalidOperation {
                operation: alloc::format!("Load {node:?}: not in blue pebbles or red memory full"),
            });
        }

        self.blue_pebbles.remove(&node);
        self.red_pebbles.insert(node);
        self.io_count = self.io_count.saturating_add(1);
        self.operation_log.push(PebbleOperation::Load(node));

        Ok(())
    }

    /// R2: Move from red to blue (1 I/O).
    pub fn store(&mut self, node: T) -> Result<()> {
        if !PebbleRules::can_store(node, &self.red_pebbles) {
            return Err(PebbleError::InvalidOperation {
                operation: alloc::format!("Store {node:?}: not in red pebbles"),
            });
        }

        self.red_pebbles.remove(&node);
        self.blue_pebbles.insert(node);
        self.io_count = self.io_count.saturating_add(1);
        self.operation_log.push(PebbleOperation::Store(node));

        Ok(())
    }

    /// R3: Compute (all dependencies must be red).
    pub fn compute(&mut self, node: T, dependencies: &[T]) -> Result<()> {
        let has_missing_deps = dependencies.iter().any(|d| !self.red_pebbles.contains(d));
        if has_missing_deps {
            return Err(PebbleError::InvalidOperation {
                operation: alloc::format!("Compute {node:?}: dependencies not in red pebbles"),
            });
        }

        let space_ok =
            self.red_pebbles.contains(&node) || self.red_pebbles.len() < self.max_red_pebbles;
        if !space_ok {
            return Err(PebbleError::FastMemoryExhausted {
                current: self.red_pebbles.len(),
                max_size: self.max_red_pebbles,
            });
        }

        self.red_pebbles.insert(node);
        self.operation_log.push(PebbleOperation::Compute(node));

        Ok(())
    }

    /// R4: Delete red pebble.
    pub fn delete(&mut self, node: T) -> Result<()> {
        if !PebbleRules::can_delete(node, &self.red_pebbles) {
            return Err(PebbleError::InvalidOperation {
                operation: alloc::format!("Delete {node:?}: not in red pebbles"),
            });
        }

        self.red_pebbles.remove(&node);
        self.operation_log.push(PebbleOperation::Delete(node));

        Ok(())
    }

    #[inline]
    pub fn io_count(&self) -> usize {
        self.io_count
    }

    #[inline]
    pub fn red_utilization(&self) -> f64 {
        if self.max_red_pebbles == 0 {
            return 0.0;
        }
        self.red_pebbles.len() as f64 / self.max_red_pebbles as f64
    }

    #[inline]
    pub fn red_count(&self) -> usize {
        self.red_pebbles.len()
    }

    #[inline]
    pub fn blue_count(&self) -> usize {
        self.blue_pebbles.len()
    }

    #[inline]
    pub fn max_red(&self) -> usize {
        self.max_red_pebbles
    }

    #[inline]
    pub fn is_red(&self, node: T) -> bool {
        self.red_pebbles.contains(&node)
    }

    #[inline]
    pub fn is_blue(&self, node: T) -> bool {
        self.blue_pebbles.contains(&node)
    }

    #[must_use = "validation result should be checked"]
    pub fn validate_invariants(&self) -> Result<()> {
        if self.red_pebbles.len() > self.max_red_pebbles {
            return Err(PebbleError::FastMemoryExhausted {
                current: self.red_pebbles.len(),
                max_size: self.max_red_pebbles,
            });
        }

        for node in &self.red_pebbles {
            if self.blue_pebbles.contains(node) {
                return Err(PebbleError::InvalidOperation {
                    operation: alloc::format!("Node {node:?} has both red and blue pebbles"),
                });
            }
        }

        Ok(())
    }

    pub fn operation_log(&self) -> &[PebbleOperation<T>] {
        &self.operation_log
    }

    pub fn clear_log(&mut self) {
        self.operation_log.clear();
    }

    pub fn red_pebbles(&self) -> &HashSet<T> {
        &self.red_pebbles
    }

    pub fn blue_pebbles(&self) -> &HashSet<T> {
        &self.blue_pebbles
    }

    // Shadow tracking primitives for PebbleManager's debug invariant checker.
    // These bypass precondition checks — the manager knows the correct
    // sequencing, the game just mirrors the resulting state.

    /// Place node in red unconditionally. Removes from blue if present.
    #[cfg(debug_assertions)]
    pub(crate) fn place_red(&mut self, node: T) {
        self.blue_pebbles.remove(&node);
        self.red_pebbles.insert(node);
    }

    /// Place node in blue unconditionally. Removes from red if present.
    #[cfg(debug_assertions)]
    pub(crate) fn place_blue(&mut self, node: T) {
        self.red_pebbles.remove(&node);
        self.blue_pebbles.insert(node);
    }

    /// Move red→blue. Increments I/O.
    #[cfg(debug_assertions)]
    #[allow(dead_code)] // used only in #[cfg(not(feature = "cold-buffer"))]
    pub(crate) fn move_to_blue(&mut self, node: T) {
        assert!(
            self.red_pebbles.remove(&node),
            "move_to_blue: {node:?} not in red"
        );
        self.blue_pebbles.insert(node);
        self.io_count = self.io_count.saturating_add(1);
    }

    /// Move blue→red. Increments I/O.
    #[cfg(debug_assertions)]
    pub(crate) fn move_to_red(&mut self, node: T) {
        assert!(
            self.blue_pebbles.remove(&node),
            "move_to_red: {node:?} not in blue"
        );
        self.red_pebbles.insert(node);
        self.io_count = self.io_count.saturating_add(1);
    }

    /// Remove node from whichever set it's in.
    #[cfg(debug_assertions)]
    pub(crate) fn remove_node(&mut self, node: T) {
        self.red_pebbles.remove(&node);
        self.blue_pebbles.remove(&node);
    }
}
