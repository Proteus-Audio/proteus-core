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
