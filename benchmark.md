# Input preparation

- Decoding and `.prot` settings load: 66.991 ms
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
| No effects        |       0 |      2.998 |        0.600 |    0.518 |    0.721 |           0.000x |
| DelayReverb       |       1 |     40.436 |        8.087 |    6.816 |   11.220 |           0.000x |
| DiffusionReverb   |       1 |      5.131 |        1.026 |    0.943 |    1.078 |           0.000x |
| ConvolutionReverb |       1 |  14330.563 |     2866.113 | 2687.684 | 3058.346 |           0.080x |
| LowPassFilter     |       1 |     97.570 |       19.514 |   16.047 |   24.114 |           0.001x |
| HighPassFilter    |       1 |     89.742 |       17.948 |   15.703 |   21.335 |           0.001x |
| Distortion        |       1 |     28.766 |        5.753 |    4.928 |    7.974 |           0.000x |
| Gain              |       1 |     22.061 |        4.412 |    3.597 |    5.062 |           0.000x |
| Compressor        |       1 |    242.895 |       48.579 |   34.891 |   67.159 |           0.001x |
| Limiter           |       1 |    319.037 |       63.807 |   59.446 |   66.820 |           0.002x |
| MultibandEq       |       1 |    318.498 |       63.700 |   58.809 |   77.493 |           0.002x |
| Pan               |       1 |     19.052 |        3.810 |    2.815 |    4.415 |           0.000x |
| Project chain     |       4 |  19625.907 |     3925.181 | 3184.752 | 4357.953 |           0.110x |
