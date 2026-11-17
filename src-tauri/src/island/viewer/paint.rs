//! Editor marks are drawn by Direct2D with one code on screen and in the file, so what is seen in the editor
//! is what gets saved. Coordinates are shot pixels, on top of the transform already set on the render target.

use crate::capture::Image;
use crate::shot::edit::{self, Area, MARKER_ALPHA, Mark, Point, Rgb};
use windows::{
    Win32::{
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
                    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
                    D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT,
                },
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_BITMAP_PROPERTIES,
                D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID, D2D1_ELLIPSE,
                D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
                D2D1_LINE_JOIN_ROUND, D2D1_QUADRATIC_BEZIER_SEGMENT, D2D1_RENDER_TARGET_PROPERTIES,
                D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
                D2D1_STROKE_STYLE_PROPERTIES, D2D1CreateFactory, ID2D1Factory, ID2D1PathGeometry,
                ID2D1RenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Imaging::{CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory},
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::Result,
};
use windows_numerics::Vector2;

fn pt((x, y): Point) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn solid(rt: &ID2D1RenderTarget, [r, g, b]: Rgb, alpha: f32) -> Result<ID2D1SolidColorBrush> {
    unsafe {
        rt.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: f32::from(r) / 255.0,
                g: f32::from(g) / 255.0,
                b: f32::from(b) / 255.0,
                a: alpha,
            },
            None,
        )
    }
}

fn round(factory: &ID2D1Factory) -> Result<ID2D1StrokeStyle> {
    unsafe {
        factory.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 1.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            },
            None,
        )
    }
}

fn rect(area: Area) -> D2D_RECT_F {
    D2D_RECT_F {
        left: area.x,
        top: area.y,
        right: area.right(),
        bottom: area.bottom(),
    }
}

/// A smoothed hand-drawn line: the kinks between mouse points become arcs (curves through segment midpoints).
fn smooth(factory: &ID2D1Factory, points: &[Point]) -> Result<ID2D1PathGeometry> {
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(pt(points[0]), D2D1_FIGURE_BEGIN_HOLLOW);
        for pair in points[1..].windows(2) {
            let (a, b) = (pair[0], pair[1]);
            sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                point1: pt(a),
                point2: pt(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)),
            });
        }
        sink.AddLine(pt(points[points.len() - 1]));
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        Ok(path)
    }
}

fn triangle(factory: &ID2D1Factory, points: [Point; 3]) -> Result<ID2D1PathGeometry> {
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(pt(points[0]), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLine(pt(points[1]));
        sink.AddLine(pt(points[2]));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(path)
    }
}

/// A crop draws nothing: it is applied on save.
pub fn draw(rt: &ID2D1RenderTarget, mark: &Mark) -> Result<()> {
    unsafe {
        let factory = rt.GetFactory()?;
        match mark {
            Mark::Stroke {
                points,
                color,
                width,
                marker,
            } => {
                let alpha = if *marker { MARKER_ALPHA } else { 1.0 };
                let brush = solid(rt, *color, alpha)?;
                if let [only] = points.as_slice() {
                    rt.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: pt(*only),
                            radiusX: width / 2.0,
                            radiusY: width / 2.0,
                        },
                        &brush,
                    );
                } else if !points.is_empty() {
                    // The whole line is one shape: a translucent marker doesn't darken where the line crosses itself.
                    let path = smooth(&factory, points)?;
                    rt.DrawGeometry(&path, &brush, *width, &round(&factory)?);
                }
            }
            Mark::Arrow {
                from,
                to,
                color,
                width,
            } => {
                let Some(head) = edit::arrow_head(*from, *to, *width) else {
                    return Ok(());
                };
                let brush = solid(rt, *color, 1.0)?;
                let style = round(&factory)?;
                rt.DrawLine(pt(*from), pt(head.shaft_end), &brush, *width, &style);
                let tip = triangle(&factory, head.points)?;
                rt.FillGeometry(&tip, &brush, None);
                // A thin outline of the same color slightly rounds the sharp corners of the arrow head.
                rt.DrawGeometry(&tip, &brush, (width * 0.4).min(3.0), &style);
            }
            Mark::Frame { area, color, width } => {
                let brush = solid(rt, *color, 1.0)?;
                rt.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(*area),
                        radiusX: *width,
                        radiusY: *width,
                    },
                    &brush,
                    *width,
                    &round(&factory)?,
                );
            }
            Mark::Pixels { area, patch } => {
                let bitmap = rt.CreateBitmap(
                    D2D_SIZE_U {
                        width: patch.width,
                        height: patch.height,
                    },
                    Some(patch.pixels.as_ptr() as *const _),
                    patch.width * 4,
                    &D2D1_BITMAP_PROPERTIES {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                    },
                )?;
                rt.DrawBitmap(
                    &bitmap,
                    Some(&rect(*area)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    None,
                );
            }
            Mark::Crop(_) => {}
        }
        Ok(())
    }
}

/// The shot with marks, cropped by the last crop: what is written to the file and put in the clipboard.
pub fn flatten(image: &Image, marks: &[&Mark]) -> Result<Image> {
    let (w, h) = (image.width, image.height);
    let mut pixels = vec![0u8; image.pixels.len()];
    unsafe {
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        // The shot's alpha is 255 everywhere, so the premultiplied format has the same bytes.
        let bitmap =
            wic.CreateBitmapFromMemory(w, h, &GUID_WICPixelFormat32bppPBGRA, w * 4, &image.pixels)?;
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let rt = factory.CreateWicBitmapRenderTarget(
            &bitmap,
            &D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            },
        )?;
        rt.BeginDraw();
        // Drawing is ended in any case, otherwise the surface stays busy.
        let drawn = marks.iter().try_for_each(|mark| draw(&rt, mark));
        rt.EndDraw(None, None)?;
        drawn?;
        bitmap.CopyPixels(std::ptr::null(), w * 4, &mut pixels)?;
    }
    let drawn = Image {
        width: w,
        height: h,
        pixels,
    };
    let visible = edit::visible(marks.iter().copied(), Area::of(image));
    Ok(match visible.pixels(w, h) {
        Some(part) if part != (0, 0, w, h) => edit::cut_out(&drawn, part),
        _ => drawn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

    fn white(w: u32, h: u32) -> Image {
        Image {
            width: w,
            height: h,
            pixels: vec![255; (w * h * 4) as usize],
        }
    }

    fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * image.width + x) * 4) as usize;
        [
            image.pixels[i],
            image.pixels[i + 1],
            image.pixels[i + 2],
            image.pixels[i + 3],
        ]
    }

    #[test]
    fn flatten_draws_marks_and_crops() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let image = white(40, 30);
        let frame = Mark::Frame {
            area: Area {
                x: 10.0,
                y: 10.0,
                w: 20.0,
                h: 10.0,
            },
            color: [255, 0, 0],
            width: 2.0,
        };
        let crop = Mark::Crop(Area {
            x: 5.0,
            y: 5.0,
            w: 30.0,
            h: 20.0,
        });
        let flat = flatten(&image, &[&frame, &crop]).expect("рисуется");
        assert_eq!((flat.width, flat.height), (30, 20));
        let [b, g, r, a] = pixel(&flat, 15, 5);
        assert!(
            r > 200 && g < 80 && b < 80,
            "красная линия, а не {b} {g} {r}"
        );
        assert_eq!(a, 255);
        assert_eq!(pixel(&flat, 15, 10), [255, 255, 255, 255]);
    }
}
