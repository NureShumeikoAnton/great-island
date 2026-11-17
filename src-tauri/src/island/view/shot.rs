//! The screenshot shutter: a 30×30 circle: a white ring fills over half a second, the dot in the center grows,
//! and at that moment the shot is taken. The island itself stays out of the frame: it is hidden from capture
//! for the shot. While the shot flies to the island the shutter stays filled: it "holds" the shot.
//! What the shot lands in is `card.rs`.

use super::super::comp::{Motion, color, v2};
use super::{Content, Scene, TEXT};
use windows::{UI::Composition::CompositionStrokeCap, core::Result};

pub const SHUTTER_MS: u32 = 500;
const SHUTTER: f32 = 30.0;
const RING_RADIUS: f32 = 6.9;
const RING_STROKE: f32 = 2.2;
const DOT_RADIUS: f32 = 3.0;

impl Scene {
    /// The shutter is one shape: track, arc and dot with a common center, no pixel offsets.
    pub(super) fn build_shutter(&self, content: &mut Content) -> Result<()> {
        content.width = SHUTTER;
        content.height = SHUTTER;
        let c = &self.comp.compositor;
        let center = v2(SHUTTER / 2.0, SHUTTER / 2.0);
        let visual = c.CreateShapeVisual()?;
        visual.SetSize(v2(SHUTTER, SHUTTER))?;
        let shapes = visual.Shapes()?;

        let circle = |radius: f32| -> Result<_> {
            let geometry = c.CreateEllipseGeometry()?;
            geometry.SetCenter(center)?;
            geometry.SetRadius(v2(radius, radius))?;
            Ok(geometry)
        };
        let track = c.CreateSpriteShapeWithGeometry(&circle(RING_RADIUS)?)?;
        track.SetStrokeBrush(&self.comp.brush(color(TEXT, 0.25))?)?;
        track.SetStrokeThickness(RING_STROKE)?;
        shapes.Append(&track)?;

        // The circle path starts at 12 o'clock and goes clockwise, like the timer rings.
        let arc_geometry = circle(RING_RADIUS)?;
        arc_geometry.SetTrimEnd(0.0)?;
        let arc = c.CreateSpriteShapeWithGeometry(&arc_geometry)?;
        arc.SetStrokeBrush(&self.comp.brush(color(TEXT, 1.0))?)?;
        arc.SetStrokeThickness(RING_STROKE)?;
        arc.SetStrokeStartCap(CompositionStrokeCap::Round)?;
        arc.SetStrokeEndCap(CompositionStrokeCap::Round)?;
        shapes.Append(&arc)?;
        self.comp.animate_scalar_from(
            &arc_geometry,
            "TrimEnd",
            0.0,
            1.0,
            Motion::linear(SHUTTER_MS),
        )?;

        let dot = c.CreateSpriteShapeWithGeometry(&circle(DOT_RADIUS)?)?;
        dot.SetFillBrush(&self.comp.brush(color(TEXT, 1.0))?)?;
        dot.SetCenterPoint(center)?;
        dot.SetScale(v2(0.0, 0.0))?;
        shapes.Append(&dot)?;
        self.comp.animate_vector2(
            &dot,
            "Scale",
            v2(1.0, 1.0),
            Motion::curve(SHUTTER_MS, 0.5, 0.0, 0.75, 0.0),
        )?;

        content.root.Children()?.InsertAtTop(&visual)
    }
}
