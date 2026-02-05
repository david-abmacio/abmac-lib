# spout TODO — Prioritized

## Phase 1: API Rename (do first — everything else builds on stable names)
- [ ] 1. Rename `Sink` trait to `Spout` (match crate name)
- [ ] 2. Rename all `*Sink` types to `*Spout` (DropSpout, CollectSpout, FnSpout, FnFlushSpout, ChannelSpout, BatchSpout, ReduceSpout, ProducerSpout)
- [ ] 3. Rename `sink()` factory function to `spout()`
- [ ] 4. Consider renaming `ChannelSpout::sender()` to `inner()` for consistency (or document why it's different)
- [ ] 5. Update all tests to use new names

## Phase 2: Metadata & Cargo.toml Fixes
- [ ] 6. Fix Cargo.toml repository URL: `spill/spill-ring` → `abmac-lib`
- [ ] 7. Add `documentation = "https://docs.rs/spout"` to Cargo.toml
- [ ] 8. Add `readme = "README.md"` to Cargo.toml
- [ ] 9. Update keywords to reflect rename (`sink` → `spout`)

## Phase 3: Bytecast Integration — Wiring
- [ ] 10. Add `bytecast` as an optional workspace dependency in Cargo.toml
- [ ] 11. Add `bytecast-macros` as an optional workspace dependency in Cargo.toml
- [ ] 12. Create `bytecast` feature flag that enables bytecast + derive support
- [ ] 13. Add feature-gated re-exports of `bytecast::{ToBytes, FromBytes, ToBytesExt, FromBytesExt}` so users don't need a separate dependency

## Phase 4: Bytecast Integration — New Types
- [ ] 14. Add `FramedSpout<T: ToBytes, S: Spout<Vec<u8>>>` — prepends framing headers (producer_id, byte length) using bytecast varint encoding
    - Use bytecast's variable-length integer encoding for length prefixes
    - Compose with ProducerSpout for tagged, framed output
- [ ] 15. Implement `ToBytes`/`FromBytes` for `BatchSpout` buffer snapshots
    - Enable checkpointing: serialize buffered items for crash recovery
    - Requires `T: ToBytes` / `T: FromBytes` bounds

## Phase 5: Tests
- [ ] 16. Add tests for `ChannelSpout` (std feature)
- [ ] 17. Add tests for `Arc<Mutex<S>>` implementation
- [ ] 18. Add tests for `send_all()` default implementation
- [ ] 19. Add concurrency/threading tests for thread-safe spouts
- [ ] 20. Add edge case tests (empty batch flush, single item batch, large batch sizes)
- [ ] 21. Add integration tests for bytecast + spout composition patterns

## Phase 6: Documentation
- [ ] 22. Add crate-level documentation (`//!`) in lib.rs explaining:
    - What the Spout pattern is and why it's useful
    - Comparison to iterators (push vs pull)
    - Composability model
    - no_std compatibility
- [ ] 23. Add doc comments to all public items:
    - `Spout` trait and methods
    - `Flush` trait
    - `DropSpout`, `CollectSpout`, `FnSpout`, `FnFlushSpout`
    - `ChannelSpout` and methods (`sender`, `into_sender`)
    - `BatchSpout` and methods (`threshold`, `buffered`, `inner`, `inner_mut`, `into_inner`)
    - `ReduceSpout` and methods (`threshold`, `buffered`, `inner`, `inner_mut`, `into_inner`)
    - `ProducerSpout` and methods (`producer_id`, `inner`, `inner_mut`, `into_inner`)
    - `FramedSpout`
- [ ] 24. Add doctests with usage examples for:
    - Basic `Spout` usage
    - `CollectSpout` example
    - `FnSpout` example
    - `BatchSpout` composition example
    - `ReduceSpout` transformation example
    - `ProducerSpout` multi-producer example
    - Bytecast composition patterns (`FnSpout` + `ToBytes`, `ReduceSpout` for batch serialization)
- [ ] 25. Create README.md with:
    - Overview and features
    - Installation instructions
    - Quick start examples
    - Feature flags documentation (`std`, `alloc`, `bytecast`)
    - License

## Phase 7: Version Bump & Release
- [ ] 26. Update version to 1.0.0

## Phase 8: Optional Enhancements (post-1.0)
- [ ] Consider adding `MapSpout<T, U, F, S>` for transformations
- [ ] Consider adding `FilterSpout<T, F, S>` for conditional forwarding
- [ ] Add performance documentation (when to use BatchSpout vs ReduceSpout)
- [ ] Add panic safety documentation
