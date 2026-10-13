//! Conventional noise reduction for process V3. V1/V2 remain in `local`.
use crate::{
    for_rows,
    local::{NrColor, NrLum},
};
use lightcraft_color::luminance_2020;
use lightcraft_develop::DevelopSettings;
use lightcraft_raster::Rgb32f;

fn unit(v: f64, default: f64) -> f32 {
    (if v.is_finite() { v / 100.0 } else { default / 100.0 }).clamp(0.0, 1.0) as f32
}

/// Shared by CPU and GPU. Radii are expressed in output pixels, relative to source resolution.
pub fn params(s: &DevelopSettings, src_long: usize, out_long: usize) -> (Option<NrLum>, Option<NrColor>) {
    if !s.section_enabled("detail") {
        return (None, None);
    }
    let lum = unit(s.detail.nr_luminance, 0.0);
    let col = unit(s.detail.nr_color, 0.0);
    let scale = (out_long as f32 / src_long.max(1) as f32).clamp(0.05, 4.0);
    let l = (lum > 0.0).then(|| {
        let detail = unit(s.detail.nr_detail, 50.0);
        let contrast = unit(s.detail.nr_contrast, 0.0);
        NrLum {
            sigma: (1.0 + 2.5 * lum) * scale.max(0.4),
            eps: 0.002 + lum * lum * 0.25 * (1.0 - 0.7 * detail) * (1.0 - 0.8 * contrast),
            k: lum.sqrt(),
        }
    });
    let c = (col > 0.0).then(|| {
        let smoothness = unit(s.detail.nr_color_smoothness, 50.0);
        let detail = unit(s.detail.nr_color_detail, 50.0);
        NrColor {
            sigma: (1.0 + 3.0 * smoothness) * scale.max(0.35),
            t: 1.0 - (-10.0 * col).exp(),
            range: (0.045 + 0.2 * col) * (1.5 - detail),
            smoothness,
        }
    });
    (l, c)
}

pub fn chroma(p: [f32; 3]) -> [f32; 3] {
    let sum = (p[0] + p[1] + p[2]).max(1e-8);
    p.map(|v| v / sum)
}

/// Prefilter single-pixel noise in the guide, then weight neighbours by colour and luminance edges.
/// Two scales handle fine speckles and broader blotches, preserving luminance detail.
pub fn color(img: &mut Rgb32f, nr: crate::local::NrColor) {
    // Store guide chromaticity and luminance together, avoiding conversions in every neighbourhood.
    let guide = lightcraft_raster::blur::gaussian(img, 0.8).map(|p| {
        let c = chroma(p);
        [c[0], c[1], c[2], luminance_2020(p).max(0.0)]
    });
    for (step, t) in [(1, nr.t), (nr.sigma.round().max(1.0) as usize, nr.t * nr.smoothness)] {
        if t <= 0.0 {
            continue;
        }
        let source = img.map(chroma);
        let (w, h) = (img.width, img.height);
        let taps: [(i64, i64, f32); 25] = std::array::from_fn(|i| {
            let (dx, dy) = ((i % 5) as i64 - 2, (i / 5) as i64 - 2);
            (dx * step as i64, dy * step as i64, 1.0 / (1.0 + (dx * dx + dy * dy) as f32))
        });
        for_rows(&mut img.data, w, |y, row| {
            for (x, p) in row.iter_mut().enumerate() {
                let center = guide.get(x, y);
                let yl = center[3];
                let range = nr.range * (0.2 / (yl + 0.02)).sqrt().clamp(1.0, 3.0);
                let mut acc = [0.0; 3];
                let mut sum = 0.0;
                for &(dx, dy, spatial) in &taps {
                    let xx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    let yy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                    let cg = guide.get(xx, yy);
                    let yn = cg[3];
                    let cd: f32 = (0..3).map(|k| (center[k] - cg[k]).powi(2)).sum();
                    let ld = (yl - yn) / (yl + yn + 0.01);
                    let distance = cd / (range * range) + ld * ld * 16.0;
                    let weight = spatial / (1.0 + distance).powi(2);
                    let c = source.get(xx, yy);
                    for k in 0..3 {
                        acc[k] += weight * c[k];
                    }
                    sum += weight;
                }
                let c0 = source.get(x, y);
                let cb = std::array::from_fn(|k| c0[k] + (acc[k] / sum.max(1e-8) - c0[k]) * t);
                let gain = luminance_2020(*p).max(0.0) / luminance_2020(cb).max(1e-8);
                *p = cb.map(|v| (v * gain).max(0.0));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::Process;

    fn error(a: &Rgb32f, b: &Rgb32f) -> f32 {
        a.data.iter().zip(&b.data).map(|(p, q)| p.iter().zip(q).map(|(u, v)| (u - v).powi(2)).sum::<f32>()).sum::<f32>() / a.len().max(1) as f32
    }

    #[test]
    fn color_noise_is_reduced_without_luminance_loss_or_colour_bleeding() {
        for exposure in [0.025, 0.2, 0.7] {
            let clean = Rgb32f::from_fn(96, 64, |x, _| if x < 48 { [exposure; 3] } else { [exposure * 2.0, exposure * 0.5, exposure * 0.1] });
            let noisy = Rgb32f::from_fn(96, 64, |x, y| {
                let p = clean.get(x, y);
                let r = (((x * 7919 + y * 104729) % 97) as f32 / 96.0 - 0.5) * exposure * 0.8;
                let b = (((x * 3571 + y * 3181) % 101) as f32 / 100.0 - 0.5) * exposure * 0.8;
                [p[0] + r, p[1] - (0.2627 * r + 0.0593 * b) / 0.678, (p[2] + b).max(0.0)]
            });
            let s = DevelopSettings::for_raw(6500.0, 0.0);
            let mut out = noisy.clone();
            crate::local::denoise(&mut out, &s, 96, 96);
            assert!(error(&out, &clean) < error(&noisy, &clean) * 0.5, "noise at {exposure}");
            for (p, q) in noisy.data.iter().zip(&out.data) {
                assert!((luminance_2020(*p) - luminance_2020(*q)).abs() < 2e-6);
                assert!(q.iter().all(|v| v.is_finite() && *v >= 0.0));
            }
            let mut edge = clean.clone();
            crate::local::denoise(&mut edge, &s, 96, 96);
            for x in [46, 49] {
                assert!((edge.get(x, 32)[0] - clean.get(x, 32)[0]).abs() < exposure * 0.08);
            }
        }
    }

    #[test]
    fn colour_detail_and_smoothness_control_the_two_scales() {
        let src = Rgb32f::from_fn(96, 64, |x, y| {
            let n = ((x as f32 * 0.63 + y as f32 * 0.8).sin() * 0.3 + (x as f32 * 0.18).sin() * 0.2) * 0.18;
            [0.18 + n, 0.18 - 0.2627 * n / 0.678, 0.18]
        });
        let mut s = DevelopSettings::for_raw(6500.0, 0.0);
        s.detail.nr_color = 60.0;
        let mut base = src.clone();
        crate::local::denoise(&mut base, &s, 96, 96);
        for (detail, smoothness) in [(100.0, 50.0), (50.0, 0.0), (50.0, 100.0)] {
            let mut out = src.clone();
            s.detail.nr_color_detail = detail;
            s.detail.nr_color_smoothness = smoothness;
            crate::local::denoise(&mut out, &s, 96, 96);
            assert!(error(&out, &base) > 1e-9, "detail {detail}, smoothness {smoothness}");
        }
    }

    #[test]
    fn legacy_colour_output_is_unchanged_and_new_parameters_are_bounded() {
        let mut s = DevelopSettings::for_raw(6500.0, 0.0);
        for process in [Process::V1, Process::V2] {
            s.process = process.version();
            let (_, nr) = crate::local::nr_params(&s, 4000, 4000);
            let nr = nr.unwrap();
            assert_eq!(nr.range, 0.0);
            assert_eq!(nr.sigma, 4.5);
            assert_eq!(nr.t, 0.1875);
        }
        s.process = Process::V3.version();
        s.detail.nr_luminance = 100.0;
        s.detail.nr_color = 100.0;
        s.detail.nr_detail = f64::NAN;
        s.detail.nr_contrast = f64::INFINITY;
        s.detail.nr_color_detail = -10000.0;
        s.detail.nr_color_smoothness = 10000.0;
        for (w, h) in [(0, 0), (1, 1), (1, 8), (7, 1)] {
            for c in [[0.0; 3], [1e-8, 0.0, 1e-8], [8.0, 2.0, 0.1]] {
                let mut img = Rgb32f::filled(w, h, c);
                crate::local::denoise(&mut img, &s, 100, 10);
                assert!(img.data.iter().flatten().all(|v| v.is_finite() && *v >= 0.0));
            }
        }
    }
}
