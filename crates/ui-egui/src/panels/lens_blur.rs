//! Lens blur panel: aperture, focus regions and undoable brush corrections.

use lightcraft_develop::{BokehShape, DevelopSettings, FocusMode};
use serde_json::json;

use super::edit::{control, sub_title};
use crate::LightcraftApp;
use crate::i18n::tr;
use crate::widgets::{register, text_button};

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &DevelopSettings) {
    let b = &d.lens_blur;
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let mut enabled = b.enabled;
            let r = ui.checkbox(&mut enabled, tr("Apply Lens Blur"));
            register(ui.ctx(), "lensBlur:enabled", r.rect);
            if r.changed() {
                let _ = app.run("lensBlur.configure", json!({"enabled": enabled}));
                if !enabled {
                    app.ui.tool.clear();
                    app.ui.lens_blur_overlay = false;
                }
            }
            if text_button(ui, "lensBlur:reset", "Reset", false).clicked() {
                let _ = app.run("develop.resetSection", json!({"section": "lensBlur"}));
                app.ui.tool.clear();
                app.ui.lens_blur_overlay = false;
            }
        });
    });
    if !b.enabled {
        return;
    }
    control(app, ui, d, "lensBlur.amount", true);
    sub_title(ui, "Bokeh");
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 4)).show(ui, |ui| {
        let items = [("Circle", "circle"), ("Bubble", "bubble"), ("Five Blade", "fiveBlade"), ("Ring", "ring"), ("Oval", "oval")];
        let shapes = [BokehShape::Circle, BokehShape::Bubble, BokehShape::FiveBlade, BokehShape::Ring, BokehShape::Oval];
        let active = shapes.iter().position(|shape| *shape == b.bokeh);
        if let Some(i) = crate::widgets::segmented(ui, "lensBlurBokeh", &items, active, 3)
            && let Some((_, key)) = items.get(i)
        {
            let _ = app.run("lensBlur.configure", json!({"bokeh": key}));
        }
    });
    control(app, ui, d, "lensBlur.boost", true);
    sub_title(ui, "Focus Protection");
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 4)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (value, name, key) in
                [(FocusMode::Subject, "Subject Estimate", "subject"), (FocusMode::Radial, "Radial Area", "radial"), (FocusMode::Manual, "Manual", "manual")]
            {
                if text_button(ui, &format!("lensBlur:mode:{key}"), name, b.mode == value).clicked() {
                    let _ = app.run("lensBlur.configure", json!({"mode": key}));
                }
            }
        });
        ui.label(
            egui::RichText::new(tr(match b.mode {
                FocusMode::Subject => "Estimate the foreground from colour edges, not physical depth. Pick a point inside the subject and refine its edges with the brush.",
                FocusMode::Radial => "Keep an oval in focus and gradually blur outside it. Click the photo to move the focus.",
                FocusMode::Manual => "Start with a blurred image. Use Focus to protect the subject and Blur to correct the painted area.",
            }))
            .small()
            .color(crate::theme::Tokens::get(ui.ctx()).text_dim),
        );
        if b.mode != FocusMode::Manual && text_button(ui, "lensBlur:pick", "Pick Focus in Photo", app.ui.tool == "lensBlurPick").clicked() {
            app.ui.tool = if app.ui.tool == "lensBlurPick" { String::new() } else { "lensBlurPick".into() };
        }
    });
    if b.mode == FocusMode::Radial {
        control(app, ui, d, "lensBlur.radiusX", true);
        control(app, ui, d, "lensBlur.radiusY", true);
    }
    if b.mode != FocusMode::Manual {
        control(app, ui, d, "lensBlur.feather", true);
        control(app, ui, d, "lensBlur.focalDistance", true);
        control(app, ui, d, "lensBlur.focalRange", true);
    }
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 4)).show(ui, |ui| {
        let r = ui.checkbox(&mut app.ui.lens_blur_overlay, tr("Show Blur Area (Blue)"));
        register(ui.ctx(), "lensBlur:overlay", r.rect);
    });
    sub_title(ui, "Refine with Brush");
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (key, label) in [("lensBlurFocus", "Focus"), ("lensBlurBlur", "Blur")] {
                if text_button(ui, key, label, app.ui.tool == key).clicked() {
                    app.ui.tool = if app.ui.tool == key { String::new() } else { key.into() };
                }
            }
            if text_button(ui, "lensBlur:clear", "Clear Brush", false).clicked() {
                let _ = app.run("lensBlur.clearBrush", json!({}));
            }
        });
        if app.ui.tool.starts_with("lensBlur") {
            ui.label(egui::RichText::new(tr("Alt switches Focus/Blur temporarily · Esc leaves the tool")).small());
        }
    });
    control(app, ui, d, "lensBlur.brushSize", true);
    control(app, ui, d, "lensBlur.brushFeather", true);
    control(app, ui, d, "lensBlur.brushFlow", true);
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 4)).show(ui, |ui| {
        let mut on = b.brush_auto_mask;
        let r = ui.checkbox(&mut on, tr("Follow Colour Edges"));
        register(ui.ctx(), "lensBlur:autoMask", r.rect);
        if r.changed() {
            let _ = app.run("lensBlur.configure", json!({"autoMask": on}));
        }
    });
    ui.add_space(8.0);
}
