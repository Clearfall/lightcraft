//! Compare V2/V3 conventional NR on a procedural image, with no personal photo or raw decoder.
//! `cargo run -p lightcraft-gpu --example conventional_nr_bench`
//! Workspace dev profiles optimize the pixel kernels; use `--release` for release measurements.
use std::{hint::black_box, sync::Arc, time::Instant};

use lightcraft_develop::{DevelopSettings, Process};
use lightcraft_pipeline::{Quality, RenderRequest, SourceInfo, StageCache};
use lightcraft_raster::Rgb32f;

fn best(mut f: impl FnMut()) -> f64 {
    let mut ms = f64::INFINITY;
    for _ in 0..3 {
        let start = Instant::now();
        f();
        ms = ms.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    ms
}

fn main() {
    let gpu = lightcraft_gpu::available();
    println!("GPU: {:?}; each value is minimum wall time of three runs", lightcraft_gpu::adapter_name());
    let src = Arc::new(Rgb32f::from_fn(6000, 4000, |x, y| {
        let n = (((x * 7919 + y * 104729) % 101) as f32 / 100.0 - 0.5) * 0.08;
        let base = if x < 3000 { 0.18 } else { 0.36 };
        [base + n, base - n * 0.4, base + n * 0.2]
    }));
    let info = SourceInfo::default();
    for (name, w, h, quality) in [("draft", 1152, 768, Quality::Draft), ("loupe", 1920, 1280, Quality::Full), ("export", 6000, 4000, Quality::Full)] {
        for process in [Process::V2, Process::V3] {
            let mut s = DevelopSettings { process: process.version(), ..Default::default() };
            s.detail.nr_luminance = 30.0;
            s.detail.nr_color = 25.0;
            let req = RenderRequest { quality, ..RenderRequest::fit(w, h) };
            let cache = StageCache::default();
            let cold = best(|| {
                cache.clear();
                black_box(lightcraft_pipeline::render_cached(&src, &info, &s, &req, &cache));
            });
            let drag = best(|| {
                s.detail.nr_color += 0.1;
                black_box(lightcraft_pipeline::render_cached(&src, &info, &s, &req, &cache));
            });
            println!("{name} {w}x{h} {process:?}: CPU cold {cold:.1} ms, NR drag {drag:.1} ms");
            cache.clear();
            if gpu {
                let mut succeeded = true;
                let ms = best(|| {
                    s.detail.nr_color += 0.1;
                    succeeded &= black_box(lightcraft_gpu::render(&src, &info, &s, &req, Some(&cache))).is_some();
                });
                println!("{name} {process:?}: GPU NR drag {ms:.1} ms, rendered={succeeded}");
            }
        }
    }
}
