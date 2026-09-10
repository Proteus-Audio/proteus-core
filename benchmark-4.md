# Input preparation

- Decoding and `.prot` settings load: 85.362 ms
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
| No effects        |       0 |      3.229 |        0.646 |    0.606 |    0.688 |           0.000x |
| DelayReverb       |       1 |     32.726 |        6.545 |    6.177 |    6.825 |           0.000x |
| DiffusionReverb   |       1 |      4.754 |        0.951 |    0.892 |    1.047 |           0.000x |
| ConvolutionReverb |       1 |   2967.499 |      593.500 |  581.157 |  600.874 |           0.017x |
| LowPassFilter     |       1 |     76.569 |       15.314 |   15.089 |   15.757 |           0.000x |
| HighPassFilter    |       1 |     77.283 |       15.457 |   15.072 |   16.051 |           0.000x |
| Distortion        |       1 |     17.399 |        3.480 |    3.419 |    3.527 |           0.000x |
| Gain              |       1 |     12.866 |        2.573 |    2.487 |    2.649 |           0.000x |
| Compressor        |       1 |    162.419 |       32.484 |   30.903 |   36.201 |           0.001x |
| Limiter           |       1 |    245.589 |       49.118 |   47.715 |   51.344 |           0.001x |
| MultibandEq       |       1 |    230.562 |       46.112 |   45.203 |   47.118 |           0.001x |
| Pan               |       1 |     13.253 |        2.651 |    2.555 |    2.737 |           0.000x |
| Project chain     |       4 |   3603.530 |      720.706 |  688.323 |  753.095 |           0.020x |
