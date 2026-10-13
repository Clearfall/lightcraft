//! Lens blur commands: configuration, focus and brush strokes are undoable edits.

use lightcraft_develop::{BrushStroke, DevelopSettings};
use serde_json::{Value, json};

use super::{CommandSpec, bad, bool_or, cmd, has_active, ok, point};
use crate::{Result, Session};

fn edit(s: &mut Session, label: &str, f: impl FnOnce(&mut DevelopSettings) -> Result<()>) -> Result<Value> {
    let id = s.active().ok_or_else(|| bad("lensBlur", "no active photo"))?;
    let mut d = (*s.develop_of(id).unwrap_or_default()).clone();
    f(&mut d)?;
    s.set_develop(id, d, label)?;
    ok()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "lensBlur.configure",
            "Configure Lens Blur",
            [],
            None,
            "{enabled?: bool, mode?: subject|radial|manual, bokeh?: circle|bubble|fiveBlade|ring|oval, autoMask?: bool}",
            has_active,
            |s, p| {
                edit(s, "Lens Blur", |d| {
                    if let Some(v) = p.get("enabled") {
                        let on = v.as_bool().ok_or_else(|| bad("lensBlur.configure", "enabled must be a boolean"))?;
                        // Older files stored zero-amount placeholders; give the first enable a visible effect.
                        if on && !d.lens_blur.enabled && d.lens_blur.amount == 0.0 {
                            d.lens_blur.amount = 50.0;
                        }
                        d.lens_blur.enabled = on;
                    }
                    if let Some(v) = p.get("autoMask") {
                        d.lens_blur.brush_auto_mask = v.as_bool().ok_or_else(|| bad("lensBlur.configure", "autoMask must be a boolean"))?;
                    }
                    if let Some(v) = p.get("mode") {
                        d.lens_blur.mode = serde_json::from_value(v.clone()).map_err(|e| bad("lensBlur.configure", e.to_string()))?;
                    }
                    if let Some(v) = p.get("bokeh") {
                        d.lens_blur.bokeh = serde_json::from_value(v.clone()).map_err(|e| bad("lensBlur.configure", e.to_string()))?;
                    }
                    Ok(())
                })
            }
        ),
        cmd!("lensBlur.focus", "Set Lens Blur Focus", [], None, "{at: [x, y]}", has_active, |s, p| {
            let at = point(p, "at")
                .filter(|p| p.x.is_finite() && p.y.is_finite() && (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y))
                .ok_or_else(|| bad("lensBlur.focus", "focus must be within the image (0..1)"))?;
            edit(s, "Set Focus", |d| {
                d.lens_blur.center = at;
                Ok(())
            })
        }),
        cmd!("lensBlur.brushStroke", "Refine Focus Area", [], None, "{points: [[x,y], ...], blur?: bool}", has_active, |s, p| {
            let points = p
                .get("points")
                .and_then(Value::as_array)
                .filter(|p| !p.is_empty() && p.len() <= 2048)
                .ok_or_else(|| bad("lensBlur.brushStroke", "a stroke requires 1..2048 points"))?;
            let points = points
                .iter()
                .map(|v| {
                    point(&json!({"p": v}), "p")
                        .filter(|p| p.x.is_finite() && p.y.is_finite() && (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y))
                        .ok_or_else(|| bad("lensBlur.brushStroke", "stroke coordinates must be within the image (0..1)"))
                })
                .collect::<Result<Vec<_>>>()?;
            edit(s, if bool_or(p, "blur", false) { "Blur Brush" } else { "Focus Brush" }, |d| {
                let b = &mut d.lens_blur;
                if b.strokes.len() >= 256 {
                    return Err(bad("lensBlur.brushStroke", "at most 256 strokes; clear the brush or undo some strokes first"));
                }
                b.strokes.push(BrushStroke {
                    points,
                    size: b.brush_size,
                    feather: b.brush_feather,
                    flow: b.brush_flow,
                    erase: bool_or(p, "blur", false),
                    auto_mask: b.brush_auto_mask,
                    ..Default::default()
                });
                Ok(())
            })
        }),
        cmd!("lensBlur.clearBrush", "Clear Focus Brush", [], None, "{}", has_active, |s, _| {
            edit(s, "Clear Focus Brush", |d| {
                d.lens_blur.strokes.clear();
                Ok(())
            })
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::{BokehShape, SettingsGroup};

    #[test]
    fn lens_blur_history_copy_reset_and_json_roundtrip() {
        let mut s = Session::with_demo();
        let id = s.active().unwrap();
        let original = s.develop_of(id).unwrap().lens_blur.clone();
        s.execute("lensBlur.configure", &json!({"enabled": true, "bokeh": "ring", "mode": "manual"})).unwrap();
        s.execute("develop.set", &json!({"control": "lensBlur.amount", "value": 70})).unwrap();
        s.execute("lensBlur.brushStroke", &json!({"points": [[0.2,0.3],[0.3,0.4]]})).unwrap();
        let d = s.develop_of(id).unwrap();
        assert_eq!(d.lens_blur.bokeh, BokehShape::Ring);
        assert_eq!(d.lens_blur.strokes.len(), 1);
        assert_eq!(DevelopSettings::from_json(&d.to_json()).unwrap(), *d);
        let copied = lightcraft_develop::extract_groups(&d, &[SettingsGroup::LensBlur]);
        let applied = DevelopSettings::default().merged(&copied).unwrap();
        assert_eq!(applied.lens_blur, d.lens_blur);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.develop_of(id).unwrap().lens_blur.strokes.is_empty());
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.develop_of(id).unwrap().lens_blur.strokes.len(), 1);
        s.execute("develop.resetSection", &json!({"section":"lensBlur"})).unwrap();
        assert_eq!(s.develop_of(id).unwrap().lens_blur, original);
    }

    #[test]
    fn lens_blur_rejects_bad_coordinates_modes_and_oversized_strokes() {
        let mut s = Session::with_demo();
        let before = s.develop_of(s.active().unwrap()).unwrap();
        for (cmd, p) in [
            ("lensBlur.focus", json!({"at":[-1,0.5]})),
            ("lensBlur.configure", json!({"enabled":true,"bokeh":"unknown"})),
            ("lensBlur.brushStroke", json!({"points":[[1e30,0.5]]})),
            ("lensBlur.brushStroke", json!({"points":[]})),
            ("lensBlur.brushStroke", json!({"points":vec![[0.5,0.5];2049]})),
        ] {
            assert!(s.execute(cmd, &p).is_err());
        }
        assert_eq!(s.develop_of(s.active().unwrap()).unwrap(), before);
    }

    #[test]
    fn lens_blur_rotation_keeps_focus_and_strokes_on_photo() {
        let mut s = Session::with_demo();
        let id = s.active().unwrap();
        s.execute("lensBlur.focus", &json!({"at":[0.2,0.3]})).unwrap();
        s.execute("lensBlur.brushStroke", &json!({"points":[[0.2,0.3]]})).unwrap();
        let before = s.develop_of(id).unwrap().lens_blur.clone();
        s.execute("photo.rotateRight", &json!({})).unwrap();
        let after = s.develop_of(id).unwrap();
        assert!((after.lens_blur.center.x - 0.7).abs() < 1e-9);
        assert!((after.lens_blur.center.y - 0.2).abs() < 1e-9);
        assert_eq!(after.lens_blur.center, after.lens_blur.strokes[0].points[0]);
        assert_eq!(after.lens_blur.radius_x, before.radius_y);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.develop_of(id).unwrap().lens_blur, before);
    }

    #[test]
    fn enabling_legacy_zero_amount_produces_visible_effect() {
        let mut s = Session::with_demo();
        s.execute("develop.merge", &json!({"settings":{"lens_blur":{"enabled":false,"amount":0.0,"focal_distance":0.0,"focal_range":0.0}}})).unwrap();
        s.execute("lensBlur.configure", &json!({"enabled":true})).unwrap();
        let b = s.develop_of(s.active().unwrap()).unwrap().lens_blur.clone();
        assert!(b.enabled && b.amount == 50.0);
        s.execute("develop.set", &json!({"control":"lensBlur.amount","value":0})).unwrap();
        assert_eq!(s.develop_of(s.active().unwrap()).unwrap().lens_blur.amount, 0.0);
    }

    #[test]
    fn legacy_disabled_placeholder_remains_unedited() {
        let d = DevelopSettings::from_json(&json!({"lens_blur":{"enabled":false,"amount":0.0,"focal_distance":0.0,"focal_range":0.0}})).unwrap();
        assert!(d.is_unedited());
    }
}
