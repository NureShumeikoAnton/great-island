//! Wrapper over Windows.UI.Composition: the scene is described once, and animation frames are computed
//! by the system compositor (DWM). The process sleeps meanwhile.
//!
//! Text is drawn by Direct2D/DirectWrite into Composition surfaces and redrawn only when it changes.

use super::glyph::{self, Glyph};
use windows::{
    Foundation::{Size, TimeSpan},
    Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat},
    System::DispatcherQueueController,
    UI::{
        Color,
        Composition::{
            AnimationDelayBehavior, AnimationIterationBehavior, CompositionColorBrush,
            CompositionDrawingSurface, CompositionEasingFunction, CompositionGraphicsDevice,
            CompositionMappingMode, CompositionObject, CompositionRadialGradientBrush, Compositor,
            ContainerVisual, Desktop::DesktopWindowTarget, SpriteVisual,
        },
    },
    Win32::{
        Foundation::{HANDLE, HMODULE, HWND, POINT},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
                    D2D1_PIXEL_FORMAT,
                },
                D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_NONE,
                D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT,
                D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1DeviceContext,
                ID2D1Factory1,
            },
            Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                ID3D11Device,
            },
            DirectWrite::{
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_FEATURE,
                DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, DWRITE_FONT_METRICS,
                DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_LINE_METRICS, DWRITE_TEXT_METRICS,
                DWRITE_TEXT_RANGE, DWriteCreateFactory, IDWriteFactory, IDWriteTextLayout,
            },
            Dxgi::{Common::DXGI_FORMAT_B8G8R8A8_UNORM, IDXGIDevice},
        },
        System::WinRT::{
            Composition::{
                ICompositionDrawingSurfaceInterop, ICompositorDesktopInterop, ICompositorInterop,
            },
            CreateDispatcherQueueController, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
            DispatcherQueueOptions,
        },
    },
    core::{BOOL, HSTRING, Interface, Result},
};
use windows_numerics::{Matrix3x2, Vector2, Vector3};

const FONT: &str = "Segoe UI";

pub fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub fn v3(x: f32, y: f32) -> Vector3 {
    Vector3 { X: x, Y: y, Z: 0.0 }
}

/// To a whole physical pixel at `scale`: text and thin lines stay sharp.
pub fn snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

/// Z scale is always 1, otherwise the transform degenerates.
pub fn scale(x: f32, y: f32) -> Vector3 {
    Vector3 { X: x, Y: y, Z: 1.0 }
}

pub fn color(hex: &str, alpha: f32) -> Color {
    let hex = hex.trim_start_matches('#');
    let channel =
        |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    Color {
        A: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        R: channel(0),
        G: channel(2),
        B: channel(4),
    }
}

#[derive(Clone, Copy)]
pub struct Motion {
    pub ms: u32,
    pub bezier: Option<[f32; 4]>,
}

impl Motion {
    pub const fn curve(ms: u32, x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self {
            ms,
            bezier: Some([x1, y1, x2, y2]),
        }
    }

    pub const fn linear(ms: u32) -> Self {
        Self { ms, bezier: None }
    }
}

fn duration(ms: u32) -> TimeSpan {
    // TimeSpan is in 100 ns units; Composition rejects zero.
    TimeSpan {
        Duration: ms.max(1) as i64 * 10_000,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub bold: bool,
    pub color: Color,
    pub tabular: bool,
}

pub struct Text {
    pub visual: SpriteVisual,
    surface: CompositionDrawingSurface,
    content: String,
    style: TextStyle,
    pub width: f32,
    pub height: f32,
}

pub struct Comp {
    pub compositor: Compositor,
    pub root: ContainerVisual,
    graphics: CompositionGraphicsDevice,
    dwrite: IDWriteFactory,
    _target: DesktopWindowTarget,
    _queue: DispatcherQueueController,
}

impl Comp {
    pub fn new(hwnd: HWND) -> Result<Self> {
        // Composition needs a dispatcher queue on the window's thread.
        let queue = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            })?
        };
        let compositor = Compositor::new()?;
        let target = unsafe {
            compositor
                .cast::<ICompositorDesktopInterop>()?
                .CreateDesktopWindowTarget(hwnd, true)?
        };
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;

        let graphics = create_graphics_device(&compositor)?;
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        Ok(Self {
            compositor,
            root,
            graphics,
            dwrite,
            _target: target,
            _queue: queue,
        })
    }

    /// Another window on the same compositor (screenshot flight): its root and target.
    /// The target keeps the window connected to Composition while it is alive.
    pub fn window_root(&self, hwnd: HWND) -> Result<(DesktopWindowTarget, ContainerVisual)> {
        let target = unsafe {
            self.compositor
                .cast::<ICompositorDesktopInterop>()?
                .CreateDesktopWindowTarget(hwnd, true)?
        };
        let root = self.compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        Ok((target, root))
    }

    pub fn container(&self) -> Result<ContainerVisual> {
        self.compositor.CreateContainerVisual()
    }

    pub fn brush(&self, c: Color) -> Result<CompositionColorBrush> {
        self.compositor.CreateColorBrushWithColor(c)
    }

    fn easing(&self, motion: Motion) -> Result<CompositionEasingFunction> {
        match motion.bezier {
            Some([x1, y1, x2, y2]) => self
                .compositor
                .CreateCubicBezierEasingFunction(v2(x1, y1), v2(x2, y2))?
                .cast(),
            None => self.compositor.CreateLinearEasingFunction()?.cast(),
        }
    }

    pub fn animate_scalar(
        &self,
        target: &impl Interface,
        property: &str,
        to: f32,
        motion: Motion,
    ) -> Result<()> {
        let anim = self.compositor.CreateScalarKeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.easing(motion)?)?;
        anim.SetDuration(duration(motion.ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    /// Starts after `delay_ms`; until then the value stays as it is.
    pub fn animate_scalar_after(
        &self,
        target: &impl Interface,
        property: &str,
        to: f32,
        motion: Motion,
        delay_ms: u32,
    ) -> Result<()> {
        let anim = self.compositor.CreateScalarKeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.easing(motion)?)?;
        anim.SetDuration(duration(motion.ms))?;
        anim.SetDelayTime(duration(delay_ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn animate_scalar_from(
        &self,
        target: &impl Interface,
        property: &str,
        from: f32,
        to: f32,
        motion: Motion,
    ) -> Result<()> {
        let anim = self.compositor.CreateScalarKeyFrameAnimation()?;
        anim.InsertKeyFrame(0.0, from)?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.easing(motion)?)?;
        anim.SetDuration(duration(motion.ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn animate_vector2(
        &self,
        target: &impl Interface,
        property: &str,
        to: Vector2,
        motion: Motion,
    ) -> Result<()> {
        let anim = self.compositor.CreateVector2KeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.easing(motion)?)?;
        anim.SetDuration(duration(motion.ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn animate_vector3(
        &self,
        target: &impl Interface,
        property: &str,
        to: Vector3,
        motion: Motion,
    ) -> Result<()> {
        let anim = self.compositor.CreateVector3KeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.easing(motion)?)?;
        anim.SetDuration(duration(motion.ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn animate_color(
        &self,
        target: &CompositionColorBrush,
        to: Color,
        motion: Motion,
    ) -> Result<()> {
        let anim = self.compositor.CreateColorKeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.easing(motion)?)?;
        anim.SetDuration(duration(motion.ms))?;
        target.StartAnimation(&HSTRING::from("Color"), &anim)
    }

    /// Binds a property to an expression: the compositor recomputes it every frame.
    /// `refs` are the objects the expression refers to, `scalars` its numeric parameters.
    pub fn bind(
        &self,
        target: &impl Interface,
        property: &str,
        expression: &str,
        refs: &[(&str, &CompositionObject)],
        scalars: &[(&str, f32)],
    ) -> Result<()> {
        let anim = self
            .compositor
            .CreateExpressionAnimationWithExpression(&HSTRING::from(expression))?;
        for (name, object) in refs {
            anim.SetReferenceParameter(&HSTRING::from(*name), *object)?;
        }
        for (name, value) in scalars {
            anim.SetScalarParameter(&HSTRING::from(*name), *value)?;
        }
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn stop(&self, target: &impl Interface, property: &str) -> Result<()> {
        target
            .cast::<CompositionObject>()?
            .StopAnimation(&HSTRING::from(property))
    }

    pub fn keyframes(
        &self,
        target: &impl Interface,
        property: &str,
        keys: &[(f32, f32)],
        ms: u32,
    ) -> Result<()> {
        let anim = self.compositor.CreateScalarKeyFrameAnimation()?;
        let linear = self.easing(Motion::linear(0))?;
        for &(at, value) in keys {
            anim.InsertKeyFrameWithEasingFunction(at.clamp(0.0, 1.0), value, &linear)?;
        }
        anim.SetDuration(duration(ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    /// One-shot vector path by keyframes: time fraction, value and the curve of the segment leading
    /// to that point (the curve's duration is unused).
    pub fn path_vector2(
        &self,
        target: &impl Interface,
        property: &str,
        keys: &[(f32, Vector2, Motion)],
        ms: u32,
    ) -> Result<()> {
        let anim = self.compositor.CreateVector2KeyFrameAnimation()?;
        for &(at, value, motion) in keys {
            anim.InsertKeyFrameWithEasingFunction(
                at.clamp(0.0, 1.0),
                value,
                &self.easing(motion)?,
            )?;
        }
        anim.SetDuration(duration(ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn path_vector3(
        &self,
        target: &impl Interface,
        property: &str,
        keys: &[(f32, Vector3, Motion)],
        ms: u32,
    ) -> Result<()> {
        let anim = self.compositor.CreateVector3KeyFrameAnimation()?;
        for &(at, value, motion) in keys {
            anim.InsertKeyFrameWithEasingFunction(
                at.clamp(0.0, 1.0),
                value,
                &self.easing(motion)?,
            )?;
        }
        anim.SetDuration(duration(ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    /// Endless looping vector animation, run by the compositor while the process sleeps.
    /// `keys` are fractions 0 to 1 and values; the first and last values must match.
    pub fn loop_vector2(
        &self,
        target: &impl Interface,
        property: &str,
        keys: &[(f32, Vector2)],
        ms: u32,
    ) -> Result<()> {
        let anim = self.compositor.CreateVector2KeyFrameAnimation()?;
        let ease = self.easing(Motion::curve(0, 0.45, 0.0, 0.55, 1.0))?;
        for &(at, value) in keys {
            anim.InsertKeyFrameWithEasingFunction(at, value, &ease)?;
        }
        anim.SetDuration(duration(ms))?;
        anim.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn loop_scalar(
        &self,
        target: &impl Interface,
        property: &str,
        keys: &[(f32, f32, Motion)],
        ms: u32,
    ) -> Result<()> {
        let anim = self.compositor.CreateScalarKeyFrameAnimation()?;
        for &(at, value, motion) in keys {
            anim.InsertKeyFrameWithEasingFunction(at, value, &self.easing(motion)?)?;
        }
        anim.SetDuration(duration(ms))?;
        anim.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    /// Keyframes with the curve of the segment leading to each one, after `delay_ms`; the first
    /// value holds during the delay. `times`: how often it plays, `None` = forever.
    #[allow(clippy::too_many_arguments)]
    pub fn scalar_keys(
        &self,
        target: &impl Interface,
        property: &str,
        keys: &[(f32, f32, Motion)],
        ms: u32,
        delay_ms: u32,
        times: Option<u32>,
    ) -> Result<()> {
        let anim = self.compositor.CreateScalarKeyFrameAnimation()?;
        for &(at, value, motion) in keys {
            anim.InsertKeyFrameWithEasingFunction(
                at.clamp(0.0, 1.0),
                value,
                &self.easing(motion)?,
            )?;
        }
        anim.SetDuration(duration(ms))?;
        if delay_ms > 0 {
            anim.SetDelayTime(duration(delay_ms))?;
            anim.SetDelayBehavior(AnimationDelayBehavior::SetInitialValueBeforeDelay)?;
        }
        match times {
            Some(n) => {
                anim.SetIterationBehavior(AnimationIterationBehavior::Count)?;
                anim.SetIterationCount(n.max(1) as i32)?;
            }
            None => anim.SetIterationBehavior(AnimationIterationBehavior::Forever)?,
        }
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn glint(
        &self,
        alpha: f32,
    ) -> Result<windows::UI::Composition::CompositionLinearGradientBrush> {
        let brush = self.compositor.CreateLinearGradientBrush()?;
        brush.SetStartPoint(v2(0.0, 0.5))?;
        brush.SetEndPoint(v2(1.0, 0.5))?;
        let stops = brush.ColorStops()?;
        for (offset, a) in [(0.0, 0.0), (0.5, alpha), (1.0, 0.0)] {
            stops.Append(
                &self
                    .compositor
                    .CreateColorGradientStopWithOffsetAndColor(offset, color("#FFFFFF", a))?,
            )?;
        }
        Ok(brush)
    }

    pub fn sheet(
        &self,
        width: u32,
        height: u32,
    ) -> Result<(CompositionDrawingSurface, SpriteVisual)> {
        let surface = self.graphics.CreateDrawingSurface(
            Size {
                Width: width.max(1) as f32,
                Height: height.max(1) as f32,
            },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )?;
        self.redraw(&surface, (0, 0, width.max(1), height.max(1)), |_| Ok(()))?;
        let visual = self.compositor.CreateSpriteVisual()?;
        visual.SetBrush(&self.compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
        visual.SetSize(v2(width.max(1) as f32, height.max(1) as f32))?;
        Ok((surface, visual))
    }

    /// Redraws a part `(x, y, w, h)` of the surface in its pixels: the part is cleared and `draw` paints in
    /// whole-surface coordinates; the excess is clipped.
    pub fn redraw(
        &self,
        surface: &CompositionDrawingSurface,
        (x, y, w, h): (u32, u32, u32, u32),
        draw: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
    ) -> Result<()> {
        let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
        let update = windows::Win32::Foundation::RECT {
            left: x as i32,
            top: y as i32,
            right: (x + w) as i32,
            bottom: (y + h) as i32,
        };
        unsafe {
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = interop.BeginDraw(Some(&update), &mut offset)?;
            dc.SetDpi(96.0, 96.0);
            dc.SetTransform(&Matrix3x2::translation(
                (offset.x - update.left) as f32,
                (offset.y - update.top) as f32,
            ));
            dc.PushAxisAlignedClip(
                &D2D_RECT_F {
                    left: x as f32,
                    top: y as f32,
                    right: (x + w) as f32,
                    bottom: (y + h) as f32,
                },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            dc.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            // Drawing is ended in any case, otherwise the surface stays busy.
            let drawn = draw(&dc);
            dc.PopAxisAlignedClip();
            interop.EndDraw()?;
            drawn
        }
    }

    pub fn radial_wash(
        &self,
        center: Vector2,
        radius: Vector2,
        c: Color,
        alpha: f32,
    ) -> Result<CompositionRadialGradientBrush> {
        let brush = self.compositor.CreateRadialGradientBrush()?;
        brush.SetMappingMode(CompositionMappingMode::Absolute)?;
        brush.SetEllipseCenter(center)?;
        brush.SetEllipseRadius(radius)?;
        let stops = brush.ColorStops()?;
        let inner = Color {
            A: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
            ..c
        };
        stops.Append(
            &self
                .compositor
                .CreateColorGradientStopWithOffsetAndColor(0.0, inner)?,
        )?;
        stops.Append(
            &self
                .compositor
                .CreateColorGradientStopWithOffsetAndColor(0.7, Color { A: 0, ..c })?,
        )?;
        Ok(brush)
    }

    pub fn picture(
        &self,
        width: f32,
        height: f32,
        scale: f32,
        draw: impl FnOnce(&ID2D1DeviceContext, &Matrix3x2) -> Result<()>,
    ) -> Result<SpriteVisual> {
        let px_w = (width * scale).ceil().max(1.0);
        let px_h = (height * scale).ceil().max(1.0);
        let surface = self.graphics.CreateDrawingSurface(
            Size {
                Width: px_w,
                Height: px_h,
            },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )?;
        let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
        unsafe {
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
            dc.SetDpi(96.0 * scale, 96.0 * scale);
            let base = Matrix3x2::translation(offset.x as f32 / scale, offset.y as f32 / scale);
            dc.SetTransform(&base);
            dc.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            // Drawing is ended in any case, otherwise the surface stays busy.
            let drawn = draw(&dc, &base);
            interop.EndDraw()?;
            drawn?;
        }
        let visual = self.compositor.CreateSpriteVisual()?;
        visual.SetBrush(&self.compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
        visual.SetSize(v2(px_w / scale, px_h / scale))?;
        Ok(visual)
    }

    /// A visual showing a swap chain another component presents into (Media Engine's video).
    /// The compositor takes each new frame by itself.
    pub fn swap_chain(&self, handle: HANDLE, size: (f32, f32)) -> Result<SpriteVisual> {
        let surface = unsafe {
            self.compositor
                .cast::<ICompositorInterop>()?
                .CreateCompositionSurfaceForHandle(handle)?
        };
        let visual = self.compositor.CreateSpriteVisual()?;
        visual.SetBrush(&self.compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
        visual.SetSize(v2(size.0, size.1))?;
        Ok(visual)
    }

    pub fn glyph(&self, glyph: Glyph, size: f32, c: Color, scale: f32) -> Result<SpriteVisual> {
        self.picture(size, size, scale, |dc, base| {
            glyph::draw(dc, base, glyph, size, c)
        })
    }

    pub fn image(
        &self,
        bgra: &[u8],
        src: (u32, u32),
        size: f32,
        round: bool,
        scale: f32,
    ) -> Result<SpriteVisual> {
        let radius = if round { size / 2.0 } else { 0.0 };
        self.image_rounded(bgra, src, (size, size), radius, scale)
    }

    /// At scale 1 and the source size the picture is in its own resolution: the visual stretches it
    /// to any size (that is how the screenshot flies).
    pub fn image_rounded(
        &self,
        bgra: &[u8],
        (src_w, src_h): (u32, u32),
        (width, height): (f32, f32),
        radius: f32,
        scale: f32,
    ) -> Result<SpriteVisual> {
        self.picture(width, height, scale, |dc, _| unsafe {
            let bitmap = dc.CreateBitmap(
                D2D_SIZE_U {
                    width: src_w,
                    height: src_h,
                },
                Some(bgra.as_ptr() as *const _),
                src_w * 4,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                },
            )?;
            let dest = D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: width,
                bottom: height,
            };
            let round = radius > 0.0;
            if round {
                let factory = dc.GetFactory()?;
                let mask = factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                    rect: dest,
                    radiusX: radius,
                    radiusY: radius,
                })?;
                dc.PushLayer(
                    &D2D1_LAYER_PARAMETERS1 {
                        contentBounds: D2D_RECT_F {
                            left: f32::MIN,
                            top: f32::MIN,
                            right: f32::MAX,
                            bottom: f32::MAX,
                        },
                        geometricMask: std::mem::ManuallyDrop::new(Some(mask.cast()?)),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: Matrix3x2::identity(),
                        opacity: 1.0,
                        opacityBrush: std::mem::ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                    },
                    None,
                );
            }
            dc.DrawBitmap(
                &bitmap,
                Some(&dest),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
            if round {
                dc.PopLayer();
            }
            Ok(())
        })
    }

    fn layout(&self, content: &str, style: TextStyle) -> Result<(IDWriteTextLayout, f32, f32)> {
        let weight = if style.bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        let wide: Vec<u16> = content.encode_utf16().collect();
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                &HSTRING::from(FONT),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size,
                &HSTRING::from("ru-RU"),
            )?;
            let layout = self
                .dwrite
                .CreateTextLayout(&wide, &format, 10_000.0, 1_000.0)?;
            if style.tabular {
                let typography = self.dwrite.CreateTypography()?;
                typography.AddFontFeature(DWRITE_FONT_FEATURE {
                    nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                    parameter: 1,
                })?;
                layout.SetTypography(
                    &typography,
                    DWRITE_TEXT_RANGE {
                        startPosition: 0,
                        length: wide.len() as u32,
                    },
                )?;
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            Ok((
                layout,
                metrics.widthIncludingTrailingWhitespace,
                metrics.height,
            ))
        }
    }

    /// Middle of capital letters from the top of the label, in logical pixels. Labels are centered on a button
    /// by it: the middle of a line with room for descenders sits visually higher, and the text looks raised.
    pub fn cap_middle(&self, style: TextStyle) -> f32 {
        self.cap_metrics(style)
            .map(|(baseline, cap, _)| baseline - cap / 2.0)
            .unwrap_or_else(|_| self.layout("Н", style).map_or(0.0, |(_, _, h)| h / 2.0))
    }

    /// Like `cap_middle`, but for all-lowercase labels: centering by the cap height leaves lowercase
    /// Cyrillic visibly low.
    pub fn x_middle(&self, style: TextStyle) -> f32 {
        self.cap_metrics(style)
            .map(|(baseline, _, x)| baseline - x / 2.0)
            .unwrap_or_else(|_| self.layout("н", style).map_or(0.0, |(_, _, h)| h / 2.0))
    }

    /// Centers by the letters (upper capitals to lower baseline), not line boxes, which leave the text visibly low.
    pub fn stack_top(&self, upper: &Text, lower: &Text, gap: f32, height: f32) -> f32 {
        let boxes = (height - (upper.height + gap + lower.height)) / 2.0;
        let above = self
            .cap_metrics(upper.style)
            .map(|(baseline, cap, _)| baseline - cap);
        let below = self
            .cap_metrics(lower.style)
            .map(|(baseline, _, _)| lower.height - baseline);
        match (above, below) {
            (Ok(above), Ok(below)) => boxes - (above - below) / 2.0,
            _ => boxes,
        }
    }

    fn cap_metrics(&self, style: TextStyle) -> Result<(f32, f32, f32)> {
        let (layout, _, _) = self.layout("Н", style)?;
        unsafe {
            let mut lines = [DWRITE_LINE_METRICS::default()];
            let mut count = 0;
            layout.GetLineMetrics(Some(&mut lines), &mut count)?;
            let mut collection = None;
            self.dwrite
                .GetSystemFontCollection(&mut collection, false)?;
            let collection = collection.ok_or_else(windows::core::Error::empty)?;
            let (mut index, mut exists) = (0, BOOL::default());
            collection.FindFamilyName(&HSTRING::from(FONT), &mut index, &mut exists)?;
            let weight = if style.bold {
                DWRITE_FONT_WEIGHT_SEMI_BOLD
            } else {
                DWRITE_FONT_WEIGHT_NORMAL
            };
            let font = collection.GetFontFamily(index)?.GetFirstMatchingFont(
                weight,
                DWRITE_FONT_STRETCH_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
            )?;
            let mut metrics = DWRITE_FONT_METRICS::default();
            font.GetMetrics(&mut metrics);
            let em = f32::from(metrics.designUnitsPerEm.max(1));
            let cap = f32::from(metrics.capHeight) / em * style.size;
            let x = f32::from(metrics.xHeight) / em * style.size;
            Ok((lines[0].baseline, cap, x))
        }
    }

    pub fn measure(&self, content: &str, style: TextStyle) -> f32 {
        self.layout(content, style)
            .map(|(_, w, _)| w)
            .unwrap_or(0.0)
    }

    pub fn fit(&self, content: &str, style: TextStyle, max_width: f32) -> String {
        if self.measure(content, style) <= max_width {
            return content.to_string();
        }
        let chars: Vec<char> = content.chars().collect();
        for len in (1..chars.len()).rev() {
            let candidate: String = chars[..len]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string()
                + "…";
            if self.measure(&candidate, style) <= max_width {
                return candidate;
            }
        }
        "…".into()
    }

    /// `scale` is the monitor's scale: text is drawn in its physical pixels.
    pub fn text(&self, content: &str, style: TextStyle, scale: f32) -> Result<Text> {
        let surface = self.graphics.CreateDrawingSurface(
            Size {
                Width: 1.0,
                Height: 1.0,
            },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )?;
        let visual = self.compositor.CreateSpriteVisual()?;
        visual.SetBrush(&self.compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
        let mut text = Text {
            visual,
            surface,
            content: String::new(),
            style,
            width: 0.0,
            height: 0.0,
        };
        self.draw_text(&mut text, content, scale)?;
        Ok(text)
    }

    pub fn set_text(&self, text: &mut Text, content: &str, scale: f32) -> Result<()> {
        if text.content == content {
            return Ok(());
        }
        self.draw_text(text, content, scale)
    }

    pub fn set_text_color(&self, text: &mut Text, c: Color, scale: f32) -> Result<()> {
        if text.style.color == c {
            return Ok(());
        }
        text.style.color = c;
        let content = std::mem::take(&mut text.content);
        self.draw_text(text, &content, scale)
    }

    fn draw_text(&self, text: &mut Text, content: &str, scale: f32) -> Result<()> {
        let (layout, width, height) = self.layout(content, text.style)?;
        let px_w = (width * scale).ceil().max(1.0);
        let px_h = (height * scale).ceil().max(1.0);
        let current = text.surface.Size()?;
        if current.Width != px_w || current.Height != px_h {
            text.surface.Resize(windows::Graphics::SizeInt32 {
                Width: px_w as i32,
                Height: px_h as i32,
            })?;
        }

        let interop: ICompositionDrawingSurfaceInterop = text.surface.cast()?;
        unsafe {
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
            dc.SetDpi(96.0 * scale, 96.0 * scale);
            dc.SetTransform(&windows_numerics::Matrix3x2::translation(
                offset.x as f32 / scale,
                offset.y as f32 / scale,
            ));
            // On a transparent background ClearType leaves color fringes, hence grayscale antialiasing.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            dc.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            let c = text.style.color;
            let brush = dc.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: c.R as f32 / 255.0,
                    g: c.G as f32 / 255.0,
                    b: c.B as f32 / 255.0,
                    a: c.A as f32 / 255.0,
                },
                None,
            )?;
            dc.DrawTextLayout(v2(0.0, 0.0), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            interop.EndDraw()?;
        }

        text.content = content.to_string();
        text.width = width;
        text.height = height;
        text.visual.SetSize(v2(px_w / scale, px_h / scale))?;
        Ok(())
    }
}

/// The GPU first, software WARP if that fails.
fn create_graphics_device(compositor: &Compositor) -> Result<CompositionGraphicsDevice> {
    let device = create_d3d_device(D3D_DRIVER_TYPE_WARP)
        .or_else(|_| create_d3d_device(D3D_DRIVER_TYPE_HARDWARE))?;
    let dxgi: IDXGIDevice = device.cast()?;
    unsafe {
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let d2d = factory.CreateDevice(&dxgi)?;
        compositor
            .cast::<ICompositorInterop>()?
            .CreateGraphicsDevice(&d2d)
    }
}

fn create_d3d_device(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(windows::core::Error::empty)
}
