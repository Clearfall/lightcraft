use super::*;
use crate::{RenderRequest, SourceInfo, StageCache, render, render_cached};
use std::sync::Arc;

fn scene(w: usize) -> Rgb32f {
    Rgb32f::from_fn(w, w, |x, y| {
        if (x as f64 / w as f64 - 0.5).hypot(y as f64 / w as f64 - 0.5) < 0.2 {
            [1.0, 0.01, 0.01]
        } else {
            [0.02, 0.08, if (x / (w / 32).max(1) + y / (w / 32).max(1)).is_multiple_of(2) { 0.2 } else { 0.7 }]
        }
    })
}

#[test]
fn keeps_focus_sharp_and_blurs_background_without_red_halo() {
    let src = scene(256);
    let mut s = DevelopSettings::default();
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 100.0;
    let frame = Frame::new(256, 256, &s, true);
    let mut out = src.clone();
    apply(&mut out, &src, &frame, &s);
    assert_eq!(out.get(128, 128), src.get(128, 128));
    assert!((out.get(20, 20)[2] - src.get(20, 20)[2]).abs() > 0.08);
    assert!(out.get(184, 128)[0] < 0.07, "the foreground red must not bleed into the background");
}

#[test]
fn disabled_and_zero_are_identity_and_constants_stay_constant() {
    let src = scene(100);
    let mut s = DevelopSettings::default();
    let f = Frame::new(100, 100, &s, true);
    for enabled in [false, true] {
        s.lens_blur.enabled = enabled;
        s.lens_blur.amount = 0.0;
        let mut img = src.clone();
        apply(&mut img, &src, &f, &s);
        assert_eq!(src, img);
    }
    s.lens_blur.amount = 100.0;
    s.lens_blur.mode = FocusMode::Manual;
    s.lens_blur.boost = 100.0;
    let src = Rgb32f::filled(100, 100, [0.8, 0.4, 0.2]);
    let mut img = src.clone();
    apply(&mut img, &src, &f, &s);
    assert!(img.data.iter().all(|p| (p[0] - 0.8).abs() < 0.0001));
}

#[test]
fn apertures_have_different_impulse_responses() {
    let src = Rgb32f::from_fn(64, 64, |x, y| if x == 32 && y == 32 { [10.0; 3] } else { [0.0; 3] });
    let mask = Plane::filled(64, 64, 1.0);
    let kinds = [BokehShape::Circle, BokehShape::Bubble, BokehShape::FiveBlade, BokehShape::Ring, BokehShape::Oval];
    let images: Vec<_> = kinds.iter().map(|k| convolve(&src, &mask, *k, 12.0, 0.0)).collect();
    for (i, a) in images.iter().enumerate() {
        assert!(a.data.iter().all(|p| p.iter().all(|v| v.is_finite() && *v >= -0.0001)));
        for b in &images[i + 1..] {
            assert_ne!(a, b);
        }
    }
    assert_eq!(images[3].get(32, 32), [0.0; 3]);
}

#[test]
fn strokes_override_focus_and_follow_crop_and_flip() {
    let src = scene(200);
    let mut s = DevelopSettings::default();
    s.lens_blur.mode = FocusMode::Manual;
    s.lens_blur.strokes.push(lightcraft_develop::BrushStroke { points: vec![Point::new(0.3, 0.4)], size: 0.08, feather: 0.0, ..Default::default() });
    let full = focus_map(&src, &Frame::new(200, 200, &s, true), &s.lens_blur, 200, 200);
    assert_eq!(full.get(60, 80), 0.0);
    assert_eq!(full.get(160, 80), 1.0);
    s.crop.geometry.rect = lightcraft_geom::Rect::new(0.2, 0.2, 0.8, 0.8);
    s.crop.flip_h = true;
    let f = Frame::new(200, 200, &s, true);
    let map = focus_map(&src, &f, &s.lens_blur, 120, 120);
    let p = f.norm_to_out(120, 120).apply(Point::new(0.3, 0.4));
    assert_eq!(bilinear(&map, p.x - 0.5, p.y - 0.5), 0.0);
    s.lens_blur.strokes.push(lightcraft_develop::BrushStroke {
        points: vec![Point::new(0.3, 0.4)],
        size: 0.03,
        feather: 0.0,
        erase: true,
        ..Default::default()
    });
    let map = focus_map(&src, &f, &s.lens_blur, 120, 120);
    assert_eq!(bilinear(&map, p.x - 0.5, p.y - 0.5), 1.0);
}

#[test]
fn cache_invalidates_on_blur_change_and_matches_uncached() {
    let src = Arc::new(scene(96));
    let mut s = DevelopSettings::default();
    let cache = StageCache::default();
    let req = RenderRequest::fit(96, 96);
    let info = SourceInfo::default();
    let a = render_cached(&src, &info, &s, &req, &cache).image;
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 100.0;
    let b = render_cached(&src, &info, &s, &req, &cache).image;
    assert_ne!(a, b);
    assert_eq!(b, render(&src, &info, &s, &req).image);
    s.set_section_enabled("lensBlur", false);
    assert_eq!(a, render_cached(&src, &info, &s, &req, &cache).image);
}

#[test]
fn nonfinite_parameters_and_tiny_images_remain_finite() {
    let mut s = DevelopSettings::default();
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 100.0;
    s.lens_blur.center = Point::new(f64::NAN, f64::INFINITY);
    s.lens_blur.feather = f64::NAN;
    s.lens_blur.strokes.push(lightcraft_develop::BrushStroke { points: vec![Point::new(f64::INFINITY, 0.0)], ..Default::default() });
    for (w, h) in [(1, 1), (1, 32), (32, 1), (32, 32)] {
        let src = Rgb32f::filled(w, h, [0.3; 3]);
        let f = Frame::new(w, h, &s, true);
        let a = focus_map(&src, &f, &s.lens_blur, w, h);
        assert!(a.data.iter().all(|v| v.is_finite()));
        let mut out = src.clone();
        apply(&mut out, &src, &f, &s);
        assert!(out.data.iter().flatten().all(|v| v.is_finite()));
    }
}

#[test]
fn color_aware_brush_protects_foreground_without_painting_blue_background() {
    let src = scene(100);
    let mut s = DevelopSettings::default();
    s.lens_blur.mode = FocusMode::Manual;
    s.lens_blur.strokes.push(lightcraft_develop::BrushStroke {
        points: vec![Point::new(0.5, 0.5)],
        size: 0.25,
        feather: 0.0,
        auto_mask: true,
        ..Default::default()
    });
    let f = Frame::new(100, 100, &s, true);
    let map = focus_map(&src, &f, &s.lens_blur, 100, 100);
    assert!(map.get(50, 50) < 0.01);
    assert!(map.get(74, 50) > 0.99);
}

#[test]
fn subject_analysis_is_reused_until_source_or_focus_changes() {
    let src = Arc::new(scene(64));
    let mut s = DevelopSettings::default();
    let f = Frame::new(64, 64, &s, true);
    let cache = StageCache::default();
    let a = cached_subject(&src, &f, &s.lens_blur, Some((&src, &cache))).unwrap();
    s.lens_blur.amount = 80.0;
    let b = cached_subject(&src, &f, &s.lens_blur, Some((&src, &cache))).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    s.lens_blur.center.x = 0.6;
    let c = cached_subject(&src, &f, &s.lens_blur, Some((&src, &cache))).unwrap();
    assert!(!Arc::ptr_eq(&b, &c));
    cache.clear();
    assert_eq!(cache.bytes(), 0);
}

#[test]
fn lens_blur_applies_to_deep_export_and_overlay_stays_out_of_deep_samples() {
    let src = scene(96);
    let mut s = DevelopSettings::default();
    let mut req = RenderRequest::fit(96, 96);
    req.depth = crate::OutputDepth::U16;
    let before = render(&src, &SourceInfo::default(), &s, &req).deep.unwrap();
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 100.0;
    let after = render(&src, &SourceInfo::default(), &s, &req).deep.unwrap();
    assert_ne!(before.samples, after.samples);
    req.overlay = crate::Overlay::LensBlur;
    assert_eq!(render(&src, &SourceInfo::default(), &s, &req).deep.unwrap().samples, after.samples);
}

#[test]
fn lens_blur_windows_match_full_renders_at_every_output_depth() {
    let src = Arc::new(scene(96));
    let mut s = DevelopSettings::default();
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 80.0;
    s.hdr.enabled = true;
    let cache = StageCache::default();
    let win = crate::PixelWindow { x: 17, y: 21, w: 48, h: 36 };
    for depth in [crate::OutputDepth::U8, crate::OutputDepth::U16, crate::OutputDepth::F32Linear, crate::OutputDepth::F32Hdr] {
        for overlay in [crate::Overlay::None, crate::Overlay::LensBlur] {
            let req = RenderRequest { depth, overlay, ..RenderRequest::fit(96, 96) };
            let full = render(&src, &SourceInfo::default(), &s, &req);
            let req = RenderRequest { window: Some(win), ..req };
            let part = render_cached(&src, &SourceInfo::default(), &s, &req, &cache);
            assert_eq!(part.image, full.image.crop(win.x, win.y, win.w, win.h));
            let clean = render(&src, &SourceInfo::default(), &s, &RenderRequest { overlay: crate::Overlay::None, ..req });
            assert_eq!(part.histogram, clean.histogram, "focus overlay must stay out of the window histogram");
            match (part.deep, full.deep) {
                (Some(part), Some(mut full)) => {
                    crop_deep(&mut full, win);
                    assert_eq!(part, full);
                    assert_eq!((part.width, part.height), (48, 36));
                }
                (None, None) => {}
                _ => panic!("window changed the output depth"),
            }
        }
    }
}

#[test]
fn lens_blur_and_hdr_overlays_have_distinct_worker_and_cache_ids() {
    let a = crate::Overlay::LensBlur;
    let b = crate::Overlay::HdrRange;
    assert_ne!(a.key(), b.key());
    assert_ne!(a.to_parts(), b.to_parts());
    let (kind, value) = a.to_parts();
    assert_eq!(kind, 6, "worker kind 5 belongs to upstream AllMasks");
    assert_eq!(crate::Overlay::from_parts(kind, value), a);
    let masks = crate::Overlay::AllMasks { view: crate::visualize::MaskView::Color, color: [255, 0, 0], opacity: 50 };
    assert_ne!(a.key(), masks.key());
    assert_ne!(a.to_parts().0, masks.to_parts().0);
    let (kind, value) = masks.to_parts();
    assert_eq!(crate::Overlay::from_parts(kind, value), masks);
}

#[test]
fn lens_blur_extreme_zoom_magnifies_native_pixels_without_a_giant_frame() {
    let src = scene(96);
    let mut s = DevelopSettings::default();
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 70.0;
    for depth in [crate::OutputDepth::U8, crate::OutputDepth::U16, crate::OutputDepth::F32Hdr] {
        let req = RenderRequest { depth, ..RenderRequest::fit(96, 96) };
        let native = render(&src, &SourceInfo::default(), &s, &req);
        let req = RenderRequest { max_w: 96_000, max_h: 96_000, window: Some(crate::PixelWindow { x: 40_000, y: 50_000, w: 256, h: 192 }), ..req };
        let zoom = render(&src, &SourceInfo::default(), &s, &req);
        assert_eq!((zoom.image.width, zoom.image.height), (256, 192));
        assert_eq!(zoom.image.get(128, 96), native.image.get(40, 50));
        assert_eq!(zoom.deep.as_ref().map(|d| (d.width, d.height)), native.deep.map(|_| (256, 192)));
    }
}
