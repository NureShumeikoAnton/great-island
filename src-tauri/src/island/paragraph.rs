//! Text that wraps, for windows laid out like web pages (the dialogs): CSS's line height,
//! letter spacing and weights, and Windows 11's "Segoe UI Variable Text" face. The island's own
//! one-line labels are `comp::Text`.

use super::comp::{Comp, v2};
use std::sync::OnceLock;
use windows::{
    UI::{Color, Composition::SpriteVisual},
    Win32::Graphics::DirectWrite::{
        DWRITE_FONT_FEATURE, DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, DWRITE_FONT_STRETCH_NORMAL,
        DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_LINE_SPACING_METHOD_UNIFORM,
        DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteTextLayout,
        IDWriteTextLayout1,
    },
    core::{HSTRING, Interface, Result},
};

/// Windows 11's text face; Windows 10 has only the older one.
const VARIABLE: &str = "Segoe UI Variable Text";
const FALLBACK: &str = "Segoe UI";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Para {
    pub size: f32,
    /// 400 normal, 600 semibold, 700 bold.
    pub weight: i32,
    pub color: Color,
    /// A fixed line height in px, like CSS `line-height`; `None` — the font's own.
    pub line: Option<f32>,
    /// Added after every character, like CSS `letter-spacing`.
    pub tracking: f32,
    pub tabular: bool,
}

pub struct Paragraph {
    pub visual: SpriteVisual,
    pub width: f32,
    pub height: f32,
}

impl Comp {
    fn face(&self) -> &'static str {
        static FACE: OnceLock<&'static str> = OnceLock::new();
        FACE.get_or_init(|| {
            if self.font_metrics(VARIABLE, 400).is_ok() {
                VARIABLE
            } else {
                FALLBACK
            }
        })
    }

    fn para_layout(
        &self,
        content: &str,
        p: &Para,
        width: Option<f32>,
    ) -> Result<(IDWriteTextLayout, f32, f32)> {
        let face = self.face();
        let wide: Vec<u16> = content.encode_utf16().collect();
        let dwrite = self.dwrite();
        unsafe {
            let format = dwrite.CreateTextFormat(
                &HSTRING::from(face),
                None,
                DWRITE_FONT_WEIGHT(p.weight),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                p.size,
                &HSTRING::from("ru-RU"),
            )?;
            if width.is_none() {
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            }
            if let Some(line) = p.line {
                // CSS puts half of what the line has beyond the font's own height above it and
                // half below: the baseline sits that far plus the ascent from the line's top.
                let metrics = self.font_metrics(face, p.weight)?;
                let em = f32::from(metrics.designUnitsPerEm.max(1));
                let ascent = f32::from(metrics.ascent) / em * p.size;
                let descent = f32::from(metrics.descent) / em * p.size;
                let baseline = (line - ascent - descent) / 2.0 + ascent;
                format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, baseline)?;
            }
            let layout =
                dwrite.CreateTextLayout(&wide, &format, width.unwrap_or(10_000.0), 10_000.0)?;
            let all = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: wide.len() as u32,
            };
            if p.tracking != 0.0 {
                layout
                    .cast::<IDWriteTextLayout1>()?
                    .SetCharacterSpacing(0.0, p.tracking, 0.0, all)?;
            }
            if p.tabular {
                let typography = dwrite.CreateTypography()?;
                typography.AddFontFeature(DWRITE_FONT_FEATURE {
                    nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                    parameter: 1,
                })?;
                layout.SetTypography(&typography, all)?;
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

    /// Width and height of `content` wrapped at `width` (`None` — one line).
    pub fn paragraph_size(&self, content: &str, p: &Para, width: Option<f32>) -> (f32, f32) {
        self.para_layout(content, p, width)
            .map(|(_, w, h)| (w, h))
            .unwrap_or((0.0, p.line.unwrap_or(p.size)))
    }

    /// `scale`: the monitor's, the text is drawn in its physical pixels.
    pub fn paragraph(
        &self,
        content: &str,
        p: &Para,
        width: Option<f32>,
        scale: f32,
    ) -> Result<Paragraph> {
        let (layout, w, h) = self.para_layout(content, p, width)?;
        let surface = self.text_surface()?;
        let size = self.draw_layout(&surface, &layout, (w, h), p.color, scale)?;
        let visual = self.compositor.CreateSpriteVisual()?;
        visual.SetBrush(&self.compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
        visual.SetSize(v2(size.X, size.Y))?;
        Ok(Paragraph {
            visual,
            width: w,
            height: h,
        })
    }
}
