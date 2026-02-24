  The ~130 ms to play trace: output stream opened is almost certainly dominated by the audio device open itself (rodio/cpal), not your mixer startup.

  Key points:

  - open_default_stream() is on the critical path in proteus-lib/src/playback/player/runtime/worker/runner.rs:92.
  - You don’t start the mix thread until after that completes (engine.start_receiver() is later at proteus-lib/src/playback/player/runtime/worker/runner.rs:111), so device-open time and decode/effect
    warmup do not overlap.

  Highest-impact improvements (for that phase / perceived startup):

  1. Reuse a persistent output stream across playback runs.

  - Right now each playback worker reopens the device in runner.rs:92.
  - If you keep one OutputStream alive in Player and only recreate/reset the Sink, you can amortize that ~130 ms.

  2. Start engine/mix work before opening the stream (overlap work).

  - Move engine.start_receiver() earlier (before open_output_stream_with_retry()), so decode startup / effect warmup can run while the OS audio backend opens.
  - This won’t reduce the output stream opened timestamp itself, but it should reduce first engine chunk received / audio_heard.

  Separate startup inefficiencies (before play trace, but worth fixing):

  1. Info::new() runs twice for container paths.

  - Prot::new() calls Info::new(...) in proteus-lib/src/container/prot.rs:81.
  - Player::new_from_path_or_paths() calls Info::new(...) again in proteus-lib/src/playback/player/mod.rs:188.
  - Info::new() does a full duration scan (get_durations_by_scan) in proteus-lib/src/container/info.rs:568 and proteus-lib/src/container/info.rs:572.
  - This is likely a major avoidable startup cost.

  2. play_settings.json parsing reopens/reparses the container.

  - Prot::load_play_settings() opens Matroska again in proteus-lib/src/container/prot.rs:246 and proteus-lib/src/container/prot.rs:247.

  3. Info and effects are printed/logged on startup.

  - println!("Info: {:?}", info) in proteus-lib/src/container/prot.rs:83 can add noise/work on hot path.

  For your second run (with convolution reverb):

  - The dominant delay is later: convolution init (258 ms) and effect warmup (306 ms), not device open.
  - Stream reuse/overlap still helps, but IR init is the bigger target there.

  If you want, I can patch the low-risk win first: remove the duplicate Info::new() call and reuse prot.info in Player::new_from_path_or_paths().


_________


The next biggest win is not generic effect warmup. It is convolution reverb initialization, specifically.

  Your logs show:

  - Non-convolution container: audio_heard ~92ms
  - Convolution container: Convolution reverb init: 257ms, effect warmup complete: 309ms, audio_heard ~372ms

  That means the dominant cost is the convolution setup itself, and warmup is mostly where that setup gets triggered.

  What “warmup” is doing now:

  - In proteus-lib/src/playback/engine/mix/runner.rs:231, startup warmup runs effect.process() on a zero buffer of min_mix_samples.
  - For convolution, process() triggers ensure_state(), which does the heavy build (Convolution reverb init) in proteus-lib/src/dsp/effects/convolution_reverb/mod.rs:150.
  - IR loading also reopens/parses the container attachment in proteus-lib/src/dsp/effects/convolution_reverb/impulse_response.rs:142.

  So:

  - Warmup is not the root problem.
  - Warmup is where the root problem currently happens.

  Is warmup needed?

  - For most effects: no, not really (your logs confirm near-zero impact).
  - For convolution: it is useful to avoid a first-audio glitch/stall, because init is expensive.
  - Removing warmup entirely would likely just move the same 257ms delay to the first real chunk.

  Best next wins (ranked)

  1. Cache/preload convolution IR state (highest impact)

  - Cache decoded impulse response (or even built convolution kernels) across playback runs.
  - Key by (container/file source, attachment/path, tail_db, channels, sample_rate if relevant).
  - This targets the 257ms directly.

  2. Stop doing full zero-buffer processing for startup warmup

  - Use the existing effect.warm_up(&context) API instead of effect.process(zero_buffer, ...).
  - That should keep convolution pre-init behavior while avoiding extra DSP work/allocation.
  - Likely saves some time (your logs suggest maybe ~40-60ms in the convolution case).

  3. Avoid reopening the .prot just to fetch the IR attachment

  - load_impulse_response_from_prot_attachment_with_tail() reparses Matroska every time.
  - If the IR attachment bytes are already known / can be cached from container load, this removes duplicate container I/O/parsing from the hot path.

  About the file-based 374ms run

  - That one looks like a different issue: packets are ready at 0-13ms, but start gate satisfied is delayed to 374ms.
  - That points to start-gate/buffering policy (mix_ready_with_min_samples(...)) rather than decode or warmup.
  - I’d treat this as the next target after convolution if file-based startup is important.

  Recommendation

  - Next patch should be: change startup warmup to call effect.warm_up() instead of effect.process() on a zero buffer.
  - Then, implement convolution IR caching (decoded IR first; kernel cache if needed).

  If you want, I can make the warmup-path change first (low risk), then profile again before tackling convolution IR caching.
