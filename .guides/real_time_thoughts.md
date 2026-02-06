Based on those numbers, this looks much more like scheduling/logic than raw DSP speed.

  ### Why

  - With reverb on, CHAIN avg ~97ms while OUT avg ~201ms.
  - With reverb off, CHAIN avg ~30ms while OUT avg ~137ms.

  So chain time is still well below output interval in both cases. If the DSP were the bottleneck, you’d
  expect CHAIN to approach or exceed OUT. It doesn’t.

  ### What the numbers imply

  - OUT is far too high (150–200ms).
    That’s a huge jitter for “samples leaving the buffer” and is likely the cause of choppy playback.
  - BUF min gets close to zero (0.01–0.02) which means you’re flirting with underruns—even if you don’t quite
    hit zero.

  This points to output pacing / buffering policy, not raw DSP speed.

  ### Most likely culprits (logic)

  1. Output loop cadence is inconsistent
     The loop sleeps (100ms) and only sends when all buffers are full. That can easily produce big, uneven OUT
     intervals.
  2. Chunk size is variable
     If length_of_smallest_buffer varies, your output packet size varies; that creates jitter.
  3. Gating by “all buffers full”
     This makes output bursty. When a track lags, playback waits, then dumps a large packet.

  ### Why reverb makes it worse

  Even though DSP is faster than output, reverb adds latency and increases the time between full-buffer
  readiness. That can cause bigger bursts, longer OUT intervals, and more jitter—even if average chain time is
  still low.

  ———

  ## What I recommend next (high confidence)

  Make output cadence fixed and decouple from “all buffers full.”

  Concrete changes:

  1. Fixed block size (e.g., 1024 or 2048 frames)
     Always output that many frames per iteration.
  2. Remove long sleeps
     Replace 100ms sleep with something like sleep(block_duration / 2) or zero sleep + event wait.
  3. Watermark-driven mixing
     If a track doesn’t have enough, pad with zeros or short crossfade.

  This will stabilize OUT near the expected block duration, which is the true target.

  ———

  ## If you want a quick proof

  I can add a metric:

  EXPECTED_OUT_MS = (block_frames / sample_rate) * 1000

  Then display:

  OUT: 160ms (expected 23ms)

  If OUT is far above expected, it’s not compute.

  ———

  ### Bottom line

  Given your stats, I’d diagnose this as buffering/scheduling logic rather than DSP speed. The audio is
  glitchy because packets are being served irregularly, not because convolution is too slow.

  If you want, I can implement the fixed‑block scheduler next (it will likely eliminate the choppiness).
