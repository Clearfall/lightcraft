# Conventional noise reduction (process V3)

V3 makes **Luminance Contrast** effective and replaces Gaussian colour smoothing with two
edge-aware chromaticity passes. Colour Detail controls tolerance across colour boundaries;
Colour Smoothness controls the wider pass. The colour stage preserves the incoming luminance.
CPU and GPU use the same parameters, and the stage cache includes the process and Contrast.

Existing V1/V2 edits keep their previous rendering. Newly imported photos use V3; an existing
photo changes only through Update to Current Process or Reset. Other rendering stages retain
V2 behaviour. This is conventional denoising, separate from the existing AI Denoise feature.
No Adobe preset, photograph, raw file or model is included.

## Evidence and limitations

Procedural regressions cover luminance-contrast response, warm-cache invalidation, old-process
stability, chroma noise reduction, colour boundaries and luminance preservation. GPU comparisons
exercise V1/V2/V3 and control extremes against CPU output; on an RTX 4060 Laptop GPU (DX12), the
largest output difference in the new cases was one 8-bit level. This is implementation agreement,
not measured Lightroom fidelity or proof of quality on every camera/noise pattern.

The reproducible example `cargo run -p lightcraft-gpu --example conventional_nr_bench` generates
a 24 MP noisy colour boundary. The following milliseconds are the minimum of three runs on that
GPU and its host CPU, using the workspace's optimized development pixel kernels. They are not
release-build numbers; other compilation was running and the image is synthetic. GPU timings
include the render call/readback; NR drag changes a control and reuses the stage cache.

| Output | Process | CPU cold | CPU NR drag | GPU NR drag |
|---|---|---:|---:|---:|
| 1152 × 768 draft | V2 | 55.9 | 34.8 | 9.4 |
| 1152 × 768 draft | V3 | 81.4 | 55.9 | 10.4 |
| 1920 × 1280 full | V2 | 108.8 | 78.2 | 25.1 |
| 1920 × 1280 full | V3 | 169.4 | 144.3 | 29.8 |
| 6000 × 4000 full | V2 | 937.7 | 776.1 | 199.2 |
| 6000 × 4000 full | V3 | 1303.6 | 1420.2 | 200.8 |

The edge-aware filtering costs substantially more CPU time. This prototype does not meet the
16 ms draft, 60 ms loupe or 1 s/24 MP CPU rendering targets in this measurement. GPU results
remain within those targets on the tested adapter. Camera-based comparisons, fidelity calibration
and CPU optimization remain review work; the parity checklist does not raise readiness estimates.

## Related proposals

This is an alternative for comparison, not an assertion that these issues have no active work:

- [#1259](https://github.com/storytold/lightcraft/pull/1259) makes Luminance Contrast effective by
  preserving more of the original difference where the guided filter changes it. This draft
  instead changes the guided filter's edge tolerance through its epsilon parameter.
- [#1348](https://github.com/storytold/lightcraft/pull/1348) proposes joint filtering of signed
  linear R−Y/B−Y differences, a noise estimate and a speck brake. This draft uses normalized RGB
  chromaticity, a fixed prefiltered guide and two 25-tap neighbourhoods; it has no fitted noise
  model or isolated-speck classifier. Its synthetic evidence is narrower than camera validation.
- [#1349](https://github.com/storytold/lightcraft/pull/1349), stacked on #1348, proposes five-level
  wavelet luminance filtering, line protection and per-level noise estimates. This draft retains
  the guided luminance filter. It does not incorporate either PR's GPU band scheduler.

This draft assigns V3 relative to upstream main `68c9240`. #1348, #1349, #944, #1226 and #1383
also propose process numbers. Maintainers must reconcile numbering and inheritance in merge order;
these competing definitions of V3 must not be shipped under the same serialized number. No code
from those PRs is included here, and no issue is automatically closed by this comparison draft.
