# Input preparation

- Decoding and `.prot` settings load: 95.506 ms
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
| No effects        |       0 |      3.353 |        0.671 |    0.625 |    0.755 |           0.000x |
| DelayReverb       |       1 |     60.628 |       12.126 |    9.262 |   16.119 |           0.000x |
| DiffusionReverb   |       1 |     10.868 |        2.174 |    1.747 |    2.857 |           0.000x |
| ConvolutionReverb |       1 |  14535.750 |     2907.150 | 2793.195 | 3004.755 |           0.081x |
| LowPassFilter     |       1 |     96.569 |       19.314 |   15.390 |   21.110 |           0.001x |
| HighPassFilter    |       1 |     82.624 |       16.525 |   14.896 |   18.171 |           0.000x |
| Distortion        |       1 |     18.716 |        3.743 |    3.192 |    5.299 |           0.000x |
| Gain              |       1 |     13.322 |        2.664 |    2.415 |    3.251 |           0.000x |
| Compressor        |       1 |    176.952 |       35.390 |   32.701 |   38.187 |           0.001x |
| Limiter           |       1 |    262.790 |       52.558 |   48.777 |   56.641 |           0.001x |
| MultibandEq       |       1 |    223.222 |       44.644 |   42.397 |   45.628 |           0.001x |
| Pan               |       1 |     15.928 |        3.186 |    2.968 |    3.530 |           0.000x |
| Project chain     |       4 |  17063.723 |     3412.745 | 3181.581 | 3969.641 |           0.095x |
