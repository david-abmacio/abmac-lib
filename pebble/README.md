# pebble

O(sqrt(T)) checkpoint management using the Red-Blue Pebble Game algorithm.

## Features

- **O(sqrt(T)) space** - Bounded memory with sqrt(T) checkpoints
- **O(sqrt(T)) replay** - Reconstruct any state in at most sqrt(T) operations
- **no_std by default** - Works in embedded environments (requires `alloc`)
- **Immutable checkpoints** - Once added, checkpoints cannot be mutated, guaranteeing safe rebuilds
- **Pluggable storage** - Abstract over flash, disk, or custom backends
- **Warm recovery** - Rediscover surviving checkpoints and rebuild the DAG automatically
- **Branching** - Fork from any historical checkpoint and track branch lineage

## Usage

Implement `Checkpointable` and `CheckpointSerializer` for your type, then pass them to `PebbleManager` or use the builder ([example](examples/basic.rs)).

## How It Works

Pebble implements the Red-Blue Pebble Game, a model for space-time tradeoffs in computation:

```
Events:      [1] [2] [3] [4] [5] [6] [7] [8] [9] [10] ... [100]
                  |           |           |            |
Checkpoints:     [2]         [5]         [8]         [10]
                  |           |           |            |
             Red pebble  Red pebble  Blue pebble   Red pebble
             (in memory) (in memory) (serialized)  (in memory)
```

**Red pebbles** are checkpoints in fast memory — instant access, bounded space. **Blue pebbles** are checkpoints in storage — requires I/O, unbounded space. The algorithm automatically decides which to keep and which to evict.

## Why not an LRU?

An LRU cache evicts whichever item was *least recently used*. That works when every item is equally cheap to recompute. Checkpoints aren't — some are roots that many others depend on, some sit on critical paths, and some are cheap leaves.

Pebble uses the dependency graph to make eviction decisions:

- **An LRU** would evict a root checkpoint that hasn't been touched in a while, even though reloading it later means replaying the entire chain that depends on it.
- **Pebble** keeps that root hot because the DAG shows it has many dependents, and evicts a leaf that's cheap to rebuild instead.

The result is fewer I/O round-trips to storage for the same amount of fast memory. For tree-shaped computations, the strategy is provably within 2x of optimal I/O. For general DAGs, it stays within a 3x budget.

## Strategies

| Strategy | Space | I/O Bound | Use Case |
|----------|-------|-----------|----------|
| `TreeStrategy` | O(sqrt(T)) | 2-approximation | Tree-shaped dependencies |
| `DAGStrategy` | O(sqrt(T)) | 3x budget | General DAGs |

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `cold-buffer` | yes | `RingCold` ring-buffered cold tier |
| `cold-buffer-std` | | `ParallelCold` multi-threaded serialization (implies `cold-buffer`) |
| `bytecast` | | Zero-copy `BytecastSerializer` adapter |
| `verdict` | | `Actionable` error impls for retry integration |

## References

Pebble implements the Red-Blue Pebble Game model from Hong & Kung (1981), using leaf-count 2-approximation results from Gleinig & Hoefler (2022) for tree eviction. The sqrt(T) space bound is motivated by recent breakthroughs in space-efficient simulation.

- Hong & Kung, ["I/O complexity: The red-blue pebble game"](https://dl.acm.org/doi/10.1145/800076.802486) (STOC 1981) — the foundational I/O complexity model
- Gleinig & Hoefler, ["The red-blue pebble game on trees and DAGs with large input"](https://htor.inf.ethz.ch/publications/img/PebbleTrees.pdf) (Euro-Par 2022) — proves leaf-count is a 2-approximation for optimal I/O on trees
- Williams, ["Simulating Time in Square-Root Space"](https://eccc.weizmann.ac.il/report/2025/017/) (ECCC 2025) — proves DTIME(t) can be simulated in O(sqrt(t log t)) space
- Cook & Mertz, ["Pebble Games and Complexity"](https://dl.acm.org/doi/10.1145/3618260.3649664) (STOC 2024) — tree evaluation in O(log n * log log n) space
- Mertz, ["Reusing Space: Pebbling and Simulation"](https://iuuk.mff.cuni.cz/~iwmertz/papers/m23.reusing_space.pdf) (2023) — theoretical pebbling and simulation results
- Fortnow, ["You Need Much Less Memory Than Time"](https://blog.computationalcomplexity.org/2025/02/you-need-much-less-memory-than-time.html) — accessible overview of the sqrt space results

## License

Licensed under either of [MIT](../LICENSE-MIT) or [Apache 2.0](../LICENSE-APACHE) at your option.
