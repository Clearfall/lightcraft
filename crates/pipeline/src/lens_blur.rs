//! Lens blur: edge segmentation, manual focus and bounded aperture convolution.
//!
//! Automatic segmentation estimates foreground/background, not physical distance.
//! Radii are relative to the uncropped long edge; linear-light convolution excludes focused foreground colours.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::{Arc, Weak};

use lightcraft_develop::{BokehShape, DevelopSettings, FocusMode, LensBlur};
use lightcraft_geom::{CropGeometry, Point};
use lightcraft_raster::{Plane, Rgb32f, par_rows};

use crate::geometry::Frame;

mod segmentation;

pub(crate) struct SubjectCache {
    pub src: Weak<Rgb32f>,
    pub key: u64,
    pub plane: Arc<Plane>,
}

/// Subject analysis is independent of amount, aperture and output size, so slider drags reuse it.
pub(crate) fn cached_subject(src: &Rgb32f, frame: &Frame, b: &LensBlur, shared: Option<(&Arc<Rgb32f>, &crate::StageCache)>) -> Option<Arc<Plane>> {
    if b.mode != FocusMode::Subject {
        return None;
    }
    let mut full = frame.clone();
    full.crop = CropGeometry::default();
    full.view = None;
    full.flip_h = false;
    full.flip_v = false;
    let key = crate::hash_of((format!("{full:?}"), b.center.x.to_bits(), b.center.y.to_bits(), b.feather.to_bits()));
    if let Some((src, cache)) = shared {
        let c = cache.focus.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = c.as_ref().filter(|c| c.key == key && c.src.ptr_eq(&Arc::downgrade(src))) {
            return Some(c.plane.clone());
        }
    }
    let plane = Arc::new(subject(src, &full, b));
    if let Some((src, cache)) = shared {
        *cache.focus.lock().unwrap_or_else(|e| e.into_inner()) = Some(SubjectCache { src: Arc::downgrade(src), key, plane: plane.clone() });
    }
    Some(plane)
}

fn bounded(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

pub fn active(s: &DevelopSettings) -> bool {
    s.section_enabled("lensBlur") && s.lens_blur.enabled && bounded(s.lens_blur.amount, 0.0, 100.0, 0.0) > 0.0
}

/// Key only rendered fields: changing the next brush size does not change existing strokes.
pub fn key(s: &DevelopSettings) -> String {
    if !active(s) {
        return String::new();
    }
    let mut b = s.lens_blur.clone();
    b.brush_size = 0.0;
    b.brush_feather = 0.0;
    b.brush_flow = 0.0;
    b.brush_auto_mask = false;
    format!("{b:?}")
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn color(p: [f32; 3]) -> [f32; 3] {
    p.map(|v| {
        let v = v.max(0.0);
        (v / (1.0 + v)).sqrt()
    })
}

/// Shortest colour paths on a bounded grid: crossing an edge costs more than moving within a region.
fn distances(img: &Rgb32f, seeds: impl Iterator<Item = usize>) -> Vec<u32> {
    let mut d = vec![u32::MAX; img.len()];
    let mut queue = BinaryHeap::new();
    for i in seeds {
        if let Some(v) = d.get_mut(i) {
            *v = 0;
            queue.push(Reverse((0u32, i)));
        }
    }
    while let Some(Reverse((cost, i))) = queue.pop() {
        if d.get(i).copied() != Some(cost) {
            continue;
        }
        let x = i % img.width;
        let neighbors = [
            (x > 0).then(|| i - 1),
            (x + 1 < img.width).then_some(i + 1),
            (i >= img.width).then(|| i - img.width),
            (i + img.width < img.len()).then_some(i + img.width),
        ];
        let Some(a) = img.data.get(i) else { continue };
        for j in neighbors.into_iter().flatten() {
            let Some(b) = img.data.get(j) else { continue };
            let edge = a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum::<f32>();
            let next = cost.saturating_add(4 + (12000.0 * edge) as u32);
            if let Some(old) = d.get_mut(j)
                && next < *old
            {
                *old = next;
                queue.push(Reverse((next, j)));
            }
        }
    }
    d
}

fn subject(src: &Rgb32f, frame: &Frame, b: &LensBlur) -> Plane {
    let mut full = frame.clone();
    full.crop = CropGeometry::default();
    full.view = None;
    full.flip_h = false;
    full.flip_v = false;
    let (w, h) = full.fit(384, 384);
    let m = lightcraft_color::REC2020.to_space(&lightcraft_color::SRGB).to_f32();
    let img = full.sample(src, w, h).map(|p| color(std::array::from_fn(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2])));
    if w < 2 || h < 2 {
        return Plane::filled(w.max(1), h.max(1), 0.0);
    }
    let cx = bounded(b.center.x, 0.0, 1.0, 0.5) * w as f64;
    let cy = bounded(b.center.y, 0.0, 1.0, 0.5) * h as f64;
    let r = 0.025 * w.max(h) as f64;
    let fg = distances(&img, (0..img.len()).filter(|i| ((*i % w) as f64 + 0.5 - cx).hypot((*i / w) as f64 + 0.5 - cy) <= r.max(1.0)));
    let bg = distances(
        &img,
        (0..img.len()).filter(|i| {
            let (x, y) = (i % w, i / w);
            (x == 0 || y == 0 || x + 1 == w || y + 1 == h) && (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy) > r.max(1.0)
        }),
    );
    let initial = Plane::from_fn(w, h, |x, y| {
        let i = y * w + x;
        let f = fg.get(i).copied().unwrap_or(0) as f32;
        let back = bg.get(i).copied().unwrap_or(0) as f32;
        f / (f + back).max(1.0)
    });
    let out = segmentation::refine(&img, &initial, cx, cy);
    lightcraft_raster::blur::gaussian(&out, (bounded(b.feather, 0.0, 100.0, 30.0) as f32 * 0.015).max(0.3))
}

fn bilinear(p: &Plane, x: f64, y: f64) -> f32 {
    if p.is_empty() || p.width == 0 || p.height == 0 {
        return 0.0;
    }
    let x = x.clamp(0.0, p.width.saturating_sub(1) as f64);
    let y = y.clamp(0.0, p.height.saturating_sub(1) as f64);
    let (ix, iy) = (x as usize, y as usize);
    let (fx, fy) = ((x - ix as f64) as f32, (y - iy as f64) as f32);
    let at = |a: usize, b: usize| p.data.get(b.min(p.height - 1) * p.width + a.min(p.width - 1)).copied().unwrap_or(0.0);
    let a = at(ix, iy) * (1.0 - fx) + at(ix + 1, iy) * fx;
    let b = at(ix, iy + 1) * (1.0 - fx) + at(ix + 1, iy + 1) * fx;
    a * (1.0 - fy) + b * fy
}

/// A 0 (focused) to 1 (blurred) map at a renderer-controlled or internally bounded size.
pub fn focus_map(src: &Rgb32f, frame: &Frame, b: &LensBlur, w: usize, h: usize) -> Plane {
    let auto = cached_subject(src, frame, b, None);
    focus_map_with_subject(src, frame, b, w, h, auto.as_deref())
}

pub(crate) fn focus_map_with_subject(src: &Rgb32f, frame: &Frame, b: &LensBlur, w: usize, h: usize, auto: Option<&Plane>) -> Plane {
    let m = frame.out_to_norm(w, h);
    let center = Point::new(bounded(b.center.x, 0.0, 1.0, 0.5), bounded(b.center.y, 0.0, 1.0, 0.5));
    let rx = bounded(b.radius_x, 0.02, 1.0, 0.28);
    let ry = bounded(b.radius_y, 0.02, 1.0, 0.38);
    let feather = bounded(b.feather, 0.0, 100.0, 30.0) as f32 / 100.0;
    let distance = bounded(b.focal_distance, 0.0, 100.0, 0.0) as f32 / 100.0;
    let range = bounded(b.focal_range, 0.0, 100.0, 10.0) as f32 / 100.0;
    let mut out = Plane::new(w, h);
    par_rows(&mut out.data, w, |y, row| {
        for (x, v) in row.iter_mut().enumerate() {
            let n = m.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
            let depth = match &auto {
                Some(a) => bilinear(a, n.x * a.width as f64 - 0.5, n.y * a.height as f64 - 0.5),
                None if b.mode == FocusMode::Radial => {
                    let r = ((n.x - center.x) / rx).hypot((n.y - center.y) / ry) as f32;
                    smooth((1.0 - feather).max(0.0), 1.001, r)
                }
                None => 1.0,
            };
            *v = if b.mode == FocusMode::Manual {
                1.0
            } else {
                ((depth - distance).abs() - range).max(0.0) / (distance.max(1.0 - distance) - range).max(0.001)
            };
        }
    });
    // Scan only each stroke's bounds and cap interpolation work for untrusted settings.
    let ppl = frame.px_per_long(w);
    let to_out = frame.norm_to_out(w, h);
    let guide = b.strokes.iter().take(256).any(|s| s.auto_mask).then(|| frame.sample(src, w, h));
    for stroke in b.strokes.iter().take(256) {
        let r = (bounded(stroke.size, 0.002, 0.25, 0.03) * ppl).max(0.5);
        let hard = r * (1.0 - bounded(stroke.feather, 0.0, 100.0, 50.0) / 100.0);
        let flow = bounded(stroke.flow, 0.0, 100.0, 100.0) as f32 / 100.0;
        let target = if stroke.erase { 1.0 } else { 0.0 };
        let mut prev = None;
        let mut remaining = 4096;
        for p in
            stroke.points.iter().take(2048).filter(|p| p.x.is_finite() && p.y.is_finite() && (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y))
        {
            let q = to_out.apply(*p);
            let a: Point = prev.unwrap_or(q);
            if remaining == 0 {
                break;
            }
            let steps = ((a.dist(q) / (r * 0.3).max(1.0)).ceil().clamp(1.0, 4096.0) as usize).min(remaining);
            remaining -= steps;
            for k in 1..=steps {
                let c = a.lerp(q, k as f64 / steps as f64);
                let reference = guide
                    .as_ref()
                    .and_then(|img| img.data.get(crate::masks::dab_pixel(c, w, h)))
                    .map(|p| (lightcraft_color::luminance_2020(*p).max(1e-6).log2(), crate::masks::chromaticity(*p)));
                let x0 = (c.x - r).max(0.0) as usize;
                let x1 = ((c.x + r).ceil().max(0.0) as usize).min(w);
                let y0 = (c.y - r).max(0.0) as usize;
                let y1 = ((c.y + r).ceil().max(0.0) as usize).min(h);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let dist = (x as f64 + 0.5 - c.x).hypot(y as f64 + 0.5 - c.y);
                        let mut weight = (1.0 - smooth(hard as f32, r as f32, dist as f32)) * flow;
                        if stroke.auto_mask
                            && let Some((rl, rch)) = reference
                            && let Some(p) = guide.as_ref().and_then(|img| img.data.get(y * w + x))
                        {
                            weight *= crate::masks::auto_similarity(
                                lightcraft_color::luminance_2020(*p).max(1e-6).log2(),
                                crate::masks::chromaticity(*p),
                                rl,
                                rch,
                            );
                        }
                        if let Some(v) = out.data.get_mut(y * w + x) {
                            *v += (target - *v) * weight;
                        }
                    }
                }
            }
            prev = Some(q);
        }
    }
    out
}

fn aperture(shape: BokehShape, x: f32, y: f32) -> f32 {
    let d = x.hypot(y);
    match shape {
        BokehShape::Circle => {
            if d <= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        BokehShape::Ring => {
            if (0.68..=1.0).contains(&d) {
                1.0
            } else {
                0.0
            }
        }
        BokehShape::Bubble => {
            if d <= 1.0 {
                if d >= 0.82 { 3.0 } else { 0.4 }
            } else {
                0.0
            }
        }
        BokehShape::Oval => {
            if (x / 0.62).hypot(y) <= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        BokehShape::FiveBlade => {
            let a = (y.atan2(x) + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU / 5.0) - std::f32::consts::PI / 5.0;
            if d * a.cos() <= (std::f32::consts::PI / 5.0).cos() { 1.0 } else { 0.0 }
        }
    }
}

/// Discretize the aperture into equal-weight horizontal spans for row-prefix convolution.
fn spans(shape: BokehShape, radius: f32) -> Vec<(isize, isize, isize, f32)> {
    let r = radius.ceil().clamp(1.0, 48.0) as isize;
    let mut spans = Vec::new();
    for y in -r..=r {
        let mut start = -r;
        let mut last = 0.0;
        for x in -r..=r + 1 {
            let weight = if x <= r { aperture(shape, x as f32 / radius.max(0.5), y as f32 / radius.max(0.5)) } else { 0.0 };
            if weight != last {
                if last > 0.0 {
                    spans.push((y, start, x, last));
                }
                start = x;
                last = weight;
            }
        }
    }
    if spans.is_empty() {
        spans.push((0, 0, 1, 1.0));
    }
    spans
}

fn convolve(img: &Rgb32f, mask: &Plane, shape: BokehShape, radius: f32, boost: f32) -> Rgb32f {
    let (w, h) = (img.width, img.height);
    let stride = w + 1;
    let mut prefix = vec![[0.0f32; 4]; stride * h];
    par_rows(&mut prefix, stride, |y, row| {
        let mut acc = [0.0; 4];
        for x in 0..w {
            let p = img.data[y * w + x];
            let l = 0.2627 * p[0] + 0.678 * p[1] + 0.0593 * p[2];
            let weight = mask.data[y * w + x] * (1.0 + boost * 4.0 * (l - 0.35).clamp(0.0, 4.0));
            for c in 0..3 {
                acc[c] += p[c] * weight;
            }
            acc[3] += weight;
            row[x + 1] = acc;
        }
    });
    let spans = spans(shape, radius);
    let mut out = Rgb32f::new(w, h);
    par_rows(&mut out.data, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut sum = [0.0; 4];
            for &(dy, left, right, weight) in &spans {
                let yy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                let a = (x as isize + left).clamp(0, w as isize) as usize;
                let b = (x as isize + right).clamp(0, w as isize) as usize;
                let (a, b) = (prefix[yy * stride + a], prefix[yy * stride + b]);
                for c in 0..4 {
                    sum[c] += (b[c] - a[c]) * weight;
                }
            }
            *o = if sum[3] > 1e-5 { [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3]] } else { img.data[y * w + x] };
        }
    });
    out
}

/// Convolve at bounded resolution, then blend into the original, preserving focused pixels.
pub fn apply(img: &mut Rgb32f, src: &Rgb32f, frame: &Frame, s: &DevelopSettings) {
    if !active(s) {
        return;
    }
    let auto = cached_subject(src, frame, &s.lens_blur, None);
    apply_with_subject(img, src, frame, s, auto.as_deref());
}

pub(crate) fn apply_with_subject(img: &mut Rgb32f, src: &Rgb32f, frame: &Frame, s: &DevelopSettings, auto: Option<&Plane>) {
    if !active(s) || img.is_empty() {
        return;
    }
    let b = &s.lens_blur;
    let amount = bounded(b.amount, 0.0, 100.0, 0.0);
    let radius = frame.px_per_long(img.width) * 0.025 * amount / 100.0;
    if radius < 0.25 {
        return;
    }
    // Large radii use a reduced image; temporary buffers have a long edge of at most 1280 pixels.
    let scale = (1280.0 / img.width.max(img.height) as f64).min(24.0 / radius).min(1.0);
    let w = (img.width as f64 * scale).round().max(1.0) as usize;
    let h = (img.height as f64 * scale).round().max(1.0) as usize;
    let work = lightcraft_raster::resample::resize(img, w, h, lightcraft_raster::resample::Filter::Box);
    let mask = focus_map_with_subject(src, frame, b, w, h, auto);
    let radius = (radius * w as f64 / img.width as f64) as f32;
    let blurred = convolve(&work, &mask, b.bokeh, radius, bounded(b.boost, 0.0, 100.0, 0.0) as f32 / 100.0);
    let channels: Vec<Plane> = (0..3).map(|c| blurred.map(|p| p[c])).collect();
    let (iw, ih) = (img.width, img.height);
    par_rows(&mut img.data, iw, |y, row| {
        for (x, p) in row.iter_mut().enumerate() {
            let u = (x as f64 + 0.5) * w as f64 / iw as f64 - 0.5;
            let v = (y as f64 + 0.5) * h as f64 / ih as f64 - 0.5;
            let a = bilinear(&mask, u, v).clamp(0.0, 1.0);
            for (c, plane) in p.iter_mut().zip(&channels) {
                *c += (bilinear(plane, u, v) - *c) * a;
            }
        }
    });
}

/// Crop the already-rendered deep samples using the same clamped window as the 8-bit result.
pub(crate) fn crop_deep(img: &mut crate::DeepImage, win: crate::PixelWindow) {
    fn crop<T: Copy>(v: &[T], width: usize, win: crate::PixelWindow) -> Vec<T> {
        let mut out = Vec::new();
        for y in win.y..win.y.saturating_add(win.h) {
            let a = y.saturating_mul(width).saturating_add(win.x).saturating_mul(3);
            let b = a.saturating_add(win.w.saturating_mul(3));
            if let Some(row) = v.get(a..b) {
                out.extend_from_slice(row);
            }
        }
        out
    }
    img.samples = match &img.samples {
        crate::DeepSamples::U16(v) => crate::DeepSamples::U16(crop(v, img.width, win)),
        crate::DeepSamples::F32(v) => crate::DeepSamples::F32(crop(v, img.width, win)),
    };
    img.width = win.w;
    img.height = win.h;
}

/// A zoom window above native resolution magnifies native pixels without allocating its virtual frame.
pub(crate) fn magnify_window(out: &mut crate::Rendered, clean: Option<&lightcraft_raster::Rgba8>, win: crate::PixelWindow, fw: usize, fh: usize) {
    let (sw, sh) = (out.image.width, out.image.height);
    let at = |x: usize, y: usize| {
        let xx = ((win.x as f64 + x as f64 + 0.5) * sw as f64 / fw.max(1) as f64) as usize;
        let yy = ((win.y as f64 + y as f64 + 0.5) * sh as f64 / fh.max(1) as f64) as usize;
        (xx.min(sw.saturating_sub(1)), yy.min(sh.saturating_sub(1)))
    };
    let sample = |img: &lightcraft_raster::Rgba8| {
        lightcraft_raster::Rgba8::from_fn(win.w, win.h, |x, y| {
            let (x, y) = at(x, y);
            img.get(x, y)
        })
    };
    let histogram_image = sample(clean.unwrap_or(&out.image));
    out.histogram = lightcraft_raster::Histogram::of_srgb8(&histogram_image);
    out.image = if clean.is_some() { sample(&out.image) } else { histogram_image };
    if let Some(deep) = &mut out.deep {
        fn sample<T: Copy + Default>(v: &[T], width: usize, win: crate::PixelWindow, at: impl Fn(usize, usize) -> (usize, usize)) -> Vec<T> {
            (0..win.h)
                .flat_map(|y| (0..win.w).flat_map(move |x| (0..3).map(move |c| (x, y, c))))
                .map(|(x, y, c)| {
                    let (x, y) = at(x, y);
                    v.get((y * width + x) * 3 + c).copied().unwrap_or_default()
                })
                .collect()
        }
        deep.samples = match &deep.samples {
            crate::DeepSamples::U16(v) => crate::DeepSamples::U16(sample(v, sw, win, at)),
            crate::DeepSamples::F32(v) => crate::DeepSamples::F32(sample(v, sw, win, at)),
        };
        deep.width = win.w;
        deep.height = win.h;
    }
}

#[cfg(test)]
mod tests;
