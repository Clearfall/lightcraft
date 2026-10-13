//! Luminance Contrast must preserve edges without bringing back flat-field noise.

use crate::{RenderRequest, SourceInfo, StageCache, local, render, render_cached};
use lightcraft_develop::{DevelopSettings, Process};
use lightcraft_raster::Rgb32f;
use std::sync::Arc;

fn noisy_step() -> Rgb32f {
    Rgb32f::from_fn(128, 64, |x, y| {
        let mut n = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841);
        n ^= n >> 13;
        let noise = (n & 0xffff) as f32 / 32768.0 - 1.0;
        [0.18 * (if x < 64 { 0.8 } else { 1.2 }) * (1.0 + noise * 0.08); 3]
    })
}

#[test]
fn nr_contrast_preserves_step_contrast_and_still_smooths_flat_noise() {
    let src = noisy_step();
    let mut s = DevelopSettings::default();
    s.detail.nr_luminance = 80.0;
    let mut low = src.clone();
    local::denoise(&mut low, &s, 128, 128);
    s.detail.nr_contrast = 100.0;
    let mut high = src.clone();
    local::denoise(&mut high, &s, 128, 128);
    let edge = |img: &Rgb32f| (8..56).map(|y| img.get(65, y)[0] - img.get(62, y)[0]).sum::<f32>() / 48.0;
    let noise = |img: &Rgb32f| {
        let values: Vec<_> = (8..56).flat_map(|y| (12..45).map(move |x| img.get(x, y)[0])).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32
    };
    assert!(edge(&high) > edge(&low) * 1.10, "{} <= {}", edge(&high), edge(&low));
    assert!(noise(&high) < noise(&src) * 0.35, "flat-field noise was restored");
    assert!(high.data.iter().flatten().all(|v| v.is_finite() && *v >= 0.0));
}

#[test]
fn nr_contrast_changes_invalidate_full_and_draft_stage_caches() {
    let src = Arc::new(noisy_step());
    let info = SourceInfo::default();
    let mut s = DevelopSettings::default();
    s.detail.nr_luminance = 80.0;
    let cache = StageCache::default();
    for quality in [crate::Quality::Full, crate::Quality::Draft] {
        let req = RenderRequest { quality, ..RenderRequest::fit(128, 64) };
        s.detail.nr_contrast = 0.0;
        let before = render_cached(&src, &info, &s, &req, &cache).image;
        s.detail.nr_contrast = 100.0;
        let warm = render_cached(&src, &info, &s, &req, &cache).image;
        assert!(warm != before, "contrast slider has no effect");
        assert!(warm == render(&src, &info, &s, &req).image, "cached render differs");
    }
    s.set_section_enabled("detail", false);
    assert_eq!(
        render_cached(&src, &info, &s, &RenderRequest::fit(128, 64), &cache).image,
        render(&src, &info, &DevelopSettings::default(), &RenderRequest::fit(128, 64)).image
    );
}

#[test]
fn nr_contrast_keeps_legacy_processes_and_zero_amount_unchanged() {
    let src = noisy_step();
    for process in [Process::V1, Process::V2] {
        let mut s = DevelopSettings { process: process.version(), ..Default::default() };
        s.detail.nr_luminance = 80.0;
        let mut before = src.clone();
        local::denoise(&mut before, &s, 128, 128);
        s.detail.nr_contrast = 100.0;
        let mut after = src.clone();
        local::denoise(&mut after, &s, 128, 128);
        assert_eq!(before, after, "legacy {process:?} changed");
    }
    let mut s = DevelopSettings::default();
    s.detail.nr_contrast = 100.0;
    let mut out = src.clone();
    local::denoise(&mut out, &s, 128, 128);
    assert_eq!(out, src);
}
