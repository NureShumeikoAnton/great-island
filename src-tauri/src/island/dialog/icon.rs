//! The app's icon for a dialog's head: `assets/icon.svg` as Composition shapes, so it stays sharp
//! at any scale.

use super::super::comp::{Comp, color, v2};
use windows::{
    UI::Composition::{CompositionGeometry, CompositionMappingMode, ShapeVisual},
    core::{Interface, Result},
};

/// The drawing's own square.
const VIEW: f32 = 1024.0;

pub fn app(comp: &Comp, size: f32) -> Result<ShapeVisual> {
    let c = &comp.compositor;
    let visual = c.CreateShapeVisual()?;
    visual.SetSize(v2(VIEW, VIEW))?;
    visual.SetScale(windows_numerics::Vector3::new(
        size / VIEW,
        size / VIEW,
        1.0,
    ))?;
    let shapes = visual.Shapes()?;
    let rect = |x: f32, y: f32, w: f32, h: f32, r: f32| -> Result<CompositionGeometry> {
        let g = c.CreateRoundedRectangleGeometry()?;
        g.SetOffset(v2(x, y))?;
        g.SetSize(v2(w, h))?;
        g.SetCornerRadius(v2(r, r))?;
        g.cast()
    };
    let circle = |cx: f32, cy: f32, r: f32| -> Result<CompositionGeometry> {
        let g = c.CreateEllipseGeometry()?;
        g.SetCenter(v2(cx, cy))?;
        g.SetRadius(v2(r, r))?;
        g.cast()
    };

    let orange = c.CreateLinearGradientBrush()?;
    orange.SetMappingMode(CompositionMappingMode::Absolute)?;
    orange.SetStartPoint(v2(32.0, 32.0))?;
    orange.SetEndPoint(v2(992.0, 992.0))?;
    let stops = orange.ColorStops()?;
    for (offset, hex) in [(0.0, "#FFB340"), (1.0, "#FF7A00")] {
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(offset, color(hex, 1.0))?)?;
    }
    let tile = c.CreateSpriteShapeWithGeometry(&rect(32.0, 32.0, 960.0, 960.0, 220.0)?)?;
    tile.SetFillBrush(&orange)?;
    shapes.Append(&tile)?;

    let pill = c.CreateSpriteShapeWithGeometry(&rect(172.0, 392.0, 680.0, 240.0, 120.0)?)?;
    pill.SetFillBrush(&comp.brush(color("#000000", 1.0))?)?;
    shapes.Append(&pill)?;
    for (cx, hex) in [(312.0, "#FF9F0A"), (452.0, "#64D2FF")] {
        let ring = c.CreateSpriteShapeWithGeometry(&circle(cx, 512.0, 52.0)?)?;
        ring.SetStrokeBrush(&comp.brush(color(hex, 1.0))?)?;
        ring.SetStrokeThickness(26.0)?;
        shapes.Append(&ring)?;
    }
    let bar = c.CreateSpriteShapeWithGeometry(&rect(560.0, 490.0, 180.0, 44.0, 22.0)?)?;
    bar.SetFillBrush(&comp.brush(color("#F5F5F7", 1.0))?)?;
    shapes.Append(&bar)?;
    Ok(visual)
}
