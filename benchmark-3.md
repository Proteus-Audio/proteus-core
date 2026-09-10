# Input preparation

- Decoding and `.prot` settings load: 81.523 ms
- Audio source: first supported audio track
- The timing table below measures DSP processing of already-decoded PCM only.

# Proteus audio benchmark

- Input: `test_audio/demo-effects.prot`
- PCM: 1576512 frames, 2 channels, 44100 Hz (35.749 s)
- Timed iterations per case: 5
- Warmup iterations per case: 1
- Processing chunk: 1024 frames

| Case              | Effects | Total (ms) | Average (ms) | Min (ms) | Max (ms) | Real-time factor |
| ----------------- | ------: | ---------: | -----------: | -------: | -------: | ---------------: |
| No effects        |       0 |      3.109 |        0.622 |    0.603 |    0.645 |           0.000x |
| DelayReverb       |       1 |     34.573 |        6.915 |    6.822 |    7.079 |           0.000x |
| DiffusionReverb   |       1 |      5.035 |        1.007 |    0.942 |    1.179 |           0.000x |
| ConvolutionReverb |       1 |   3072.233 |      614.447 |  603.863 |  623.731 |           0.017x |
| LowPassFilter     |       1 |     78.453 |       15.691 |   15.373 |   16.244 |           0.000x |
| HighPassFilter    |       1 |     78.041 |       15.608 |   15.303 |   15.970 |           0.000x |
| Distortion        |       1 |     17.412 |        3.482 |    3.387 |    3.705 |           0.000x |
| Gain              |       1 |     12.932 |        2.586 |    2.410 |    2.794 |           0.000x |
| Compressor        |       1 |    162.257 |       32.451 |   31.492 |   34.429 |           0.001x |
| Limiter           |       1 |    251.676 |       50.335 |   49.076 |   50.816 |           0.001x |
| MultibandEq       |       1 |    235.203 |       47.041 |   46.001 |   50.402 |           0.001x |
| Pan               |       1 |     13.746 |        2.749 |    2.429 |    3.496 |           0.000x |
| Project chain     |       4 |   3651.424 |      730.285 |  716.985 |  759.230 |           0.020x |
