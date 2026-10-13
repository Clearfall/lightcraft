# Lens blur

Lens Blur is a non-destructive CPU effect in the Edit panel (Window > Edit Sections > Lens Blur,
Cmd/Ctrl+6). Enable **Apply Lens Blur**, set the amount and choose an aperture: Circle, Bubble,
Five Blade, Ring or Oval. Highlight Boost weights bright background samples more strongly.

Focus can be specified in three ways:

- **Subject Estimate:** pick a point inside the foreground. Colour paths and a bounded colour-model
  refinement separate it from the background. This works best with distinct foreground colours.
- **Radial Area:** place an ellipse, adjust its width, height and feather, and keep its inside sharp.
- **Manual:** start with the whole image blurred and paint the areas to keep in focus.

The Focus and Blur brushes refine any mode. Alt temporarily reverses their operation; Esc leaves
the tool. Size, feather and flow belong to each saved stroke. Follow Colour Edges limits a stroke
using colour and luminance similarity. Clear Brush removes all refinements in one undoable edit.
The blue overlay shows where blur applies; it is a view aid and is never part of an export.

The command API is `lensBlur.configure`, `lensBlur.focus`, `lensBlur.brushStroke` and
`lensBlur.clearBrush`; numeric values use the `lensBlur.*` develop controls. Changes participate in
undo/redo, copying the Lens Blur settings group, presets and LightCraft's XMP settings roundtrip.
Focus points and strokes use normalized oriented-image coordinates. Rotation transforms them;
crop and flip preserve their attachment to the picture. A stroke has at most 2,048 points and an
edit at most 256 strokes. Invalid command arguments return errors without changing the edit.

## Implementation and limits

This is a heuristic foreground/background effect, **not estimated physical depth**, a trained depth
model or a claim of Lightroom rendering parity. Complex overlapping subjects, translucent edges
and similar foreground/background colours can need manual refinement. Focus Position and Focus
Range address the estimated separation map, not metres or lens distance.

The aperture convolution runs in linear light and excludes protected foreground colours from
background samples. Its radius is relative to the uncropped image long edge. Temporary blur
buffers are capped at 1,280 pixels on the long edge; subject analysis uses at most 384 pixels and
is cached independently of aperture and amount. Changes to future brush settings do not invalidate
already-rendered strokes. The cache releases the subject map when cleared and does not own the source.

The GPU frontend deliberately uses the CPU renderer for this effect. U8, U16, linear-float and HDR
outputs share it. Region requests render the full frame and then crop, preserving the convolution
grid and its surrounding colours. Above 100% zoom, region views magnify native pixels instead of
allocating the virtual enlarged frame. This path can be slower than ordinary region
renders on large images; it is not a real-time GPU depth effect.

Tests use procedural images only: focus retention, halo avoidance, aperture impulse responses,
brushes, crop/flip/rotation, cache invalidation, invalid inputs, XMP/history, HDR/deep output and
window/full-frame agreement. No external model, photograph or Adobe asset is included.
