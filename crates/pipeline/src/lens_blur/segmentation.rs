//! Refine foreground using colour models and neighbours with bounded iterations and no external model.

use lightcraft_raster::{Plane, Rgb32f};
use std::collections::VecDeque;

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}

fn palette(samples: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if samples.is_empty() {
        return vec![[0.0; 3]];
    }
    let points: Vec<_> = samples.iter().step_by((samples.len() / 4000).max(1)).copied().collect();
    let mut centers = vec![points.iter().fold([0.0; 3], |acc, p| std::array::from_fn(|i| acc[i] + p[i] / points.len() as f32))];
    for _ in 0..5 {
        let p = points.iter().max_by(|a, b| nearest(**a, &centers).total_cmp(&nearest(**b, &centers))).copied().unwrap_or([0.0; 3]);
        centers.push(p);
    }
    for _ in 0..8 {
        let mut sums = [[0.0; 3]; 6];
        let mut counts = [0u32; 6];
        for &p in &points {
            let i = centers.iter().enumerate().min_by(|a, b| distance(p, *a.1).total_cmp(&distance(p, *b.1))).map(|v| v.0).unwrap_or(0);
            if let (Some(sum), Some(count)) = (sums.get_mut(i), counts.get_mut(i)) {
                for c in 0..3 {
                    sum[c] += p[c];
                }
                *count += 1;
            }
        }
        for ((center, sum), n) in centers.iter_mut().zip(sums).zip(counts) {
            if n > 0 {
                *center = sum.map(|v| v / n as f32);
            }
        }
    }
    centers
}

fn nearest(p: [f32; 3], centers: &[[f32; 3]]) -> f32 {
    centers.iter().map(|c| distance(p, *c)).fold(f32::MAX, f32::min)
}

struct Cluster {
    center: [f32; 3],
    inv_var: [f32; 3],
    offset: f32,
}

/// Classify using colour frequency as well, reducing contamination from rare background samples.
fn model(samples: &[[f32; 3]]) -> Vec<Cluster> {
    let centers = palette(samples);
    let mut squares = [[0.0; 3]; 6];
    let mut counts = [0u32; 6];
    for &p in samples {
        if let Some((i, center)) = centers.iter().enumerate().min_by(|a, b| distance(p, *a.1).total_cmp(&distance(p, *b.1)))
            && let (Some(sum), Some(count)) = (squares.get_mut(i), counts.get_mut(i))
        {
            for c in 0..3 {
                sum[c] += (p[c] - center[c]).powi(2);
            }
            *count += 1;
        }
    }
    centers
        .into_iter()
        .zip(squares)
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|((center, sum), n)| {
            let var = sum.map(|v| (v / n as f32).max(0.0002));
            let offset = 0.5 * var.iter().map(|v| v.ln()).sum::<f32>() - (n as f32 / samples.len().max(1) as f32).ln();
            Cluster { center, inv_var: var.map(|v| 1.0 / v), offset }
        })
        .collect()
}

fn cost(p: [f32; 3], model: &[Cluster]) -> f32 {
    model.iter().map(|c| 0.5 * (0..3).map(|i| (p[i] - c.center[i]).powi(2) * c.inv_var[i]).sum::<f32>() + c.offset).fold(1000.0, f32::min)
}

fn neighbors(i: usize, w: usize, len: usize) -> impl Iterator<Item = usize> {
    let x = i % w;
    [(x > 0).then(|| i - 1), (x + 1 < w).then_some(i + 1), (i >= w).then(|| i - w), (i + w < len).then_some(i + w)].into_iter().flatten()
}

/// Keep the region connected to the focus point, filling internal holes such as eyes or fabric patterns.
fn connected(background: &[bool], w: usize, h: usize, seed: usize) -> Vec<bool> {
    let mut foreground = vec![false; background.len()];
    let mut queue = VecDeque::new();
    if let Some(v) = foreground.get_mut(seed) {
        *v = true;
        queue.push_back(seed);
    }
    while let Some(i) = queue.pop_front() {
        for j in neighbors(i, w, background.len()) {
            if !foreground[j] && !background[j] {
                foreground[j] = true;
                queue.push_back(j);
            }
        }
    }
    let mut outside = vec![false; background.len()];
    for i in 0..background.len() {
        if !foreground[i] && (i % w == 0 || i % w + 1 == w || i / w == 0 || i / w + 1 == h) {
            outside[i] = true;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for j in neighbors(i, w, background.len()) {
            if !foreground[j] && !outside[j] {
                outside[j] = true;
                queue.push_back(j);
            }
        }
    }
    outside
}

pub(super) fn refine(img: &Rgb32f, initial: &Plane, cx: f64, cy: f64) -> Plane {
    let (w, h) = (img.width, img.height);
    let fg: Vec<_> = img.data.iter().zip(&initial.data).filter(|(_, v)| **v < 0.3).map(|(p, _)| *p).collect();
    let bg: Vec<_> = img
        .data
        .iter()
        .zip(&initial.data)
        .enumerate()
        .filter(|(i, (_, v))| **v > 0.8 && ((*i % w) as f64 - cx).hypot((*i / w) as f64 - cy) > w.max(h) as f64 * 0.2)
        .map(|(_, (p, _))| *p)
        .collect();
    if fg.is_empty() || bg.is_empty() {
        return initial.clone();
    }
    let (fg, bg) = (model(&fg), model(&bg));
    let unary: Vec<_> = img.data.iter().map(|p| cost(*p, &fg) - cost(*p, &bg)).collect();
    let mut background: Vec<_> = unary.iter().map(|v| *v > 0.0).collect();
    // Alternate checkerboard phases to avoid directional bias; at most twelve linear-time passes.
    for _ in 0..12 {
        for phase in 0..2 {
            for i in 0..img.len() {
                if (i % w + i / w) % 2 != phase {
                    continue;
                }
                let mut score = unary[i];
                for j in neighbors(i, w, img.len()) {
                    let weight = (-60.0 * distance(img.data[i], img.data[j])).exp();
                    score += weight * if background[j] { 1.0 } else { -1.0 };
                }
                background[i] = score > 0.0;
            }
        }
    }
    let seed = (cy.floor().clamp(0.0, (h - 1) as f64) as usize) * w + cx.floor().clamp(0.0, (w - 1) as f64) as usize;
    let background = connected(&background, w, h, seed);
    Plane { width: w, height: h, data: background.into_iter().map(|v| if v { 1.0 } else { 0.0 }).collect() }
}
