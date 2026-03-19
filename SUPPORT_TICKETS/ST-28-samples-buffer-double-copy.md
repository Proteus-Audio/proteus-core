# SI-28: `clone_samples_buffer` Still Copies Sample Data Twice

## Files affected

| File | Notes |
|---|---|
| `proteus-lib/src/audio/samples.rs` | `clone_samples_buffer` collects into a `Vec<f32>` and then clones that vector again |

---

## Current state

The helper still does:

```rust
let vector_samples = buffered.clone().collect::<Vec<f32>>();
let clone1 = SamplesBuffer::new(..., vector_samples.clone());
let clone2 = SamplesBuffer::new(..., vector_samples);
```

One full clone is unavoidable because two independent owned buffers are returned. The helper should
still be tightened so it is explicit about paying exactly one necessary duplication cost, not more.

### Why this matters

- This is unnecessary memory traffic in a utility that may be used on large buffers
- The current implementation obscures the true minimum-copy cost
- Small utility inefficiencies tend to survive because they look harmless

### Recommended remediation

1. Rewrite the helper so the data path is obviously "collect once, duplicate once"
2. Preserve metadata (`channels`, `sample_rate`, frame count) exactly
3. Add a comment documenting why one copy is still required to produce two owned `SamplesBuffer`s
4. Keep tests covering identical samples and identical counts after the refactor

### Acceptance criteria

- [x] `clone_samples_buffer` performs only the minimum required duplication to return two owned buffers
- [x] Existing metadata/sample-preservation tests still pass
- [x] The implementation clearly documents the unavoidable copy boundary

## Status

Complete.
