//! Windows OCR (`Windows.Media.Ocr`): part of Windows, offline, one engine per language pack
//! the user has installed.
//!
//! Recognition is a WinRT async call waited on here, so a reader lives on a worker thread in
//! the multithreaded apartment, never on the island's thread.

use crate::{
    capture::Image,
    fault::{Cause, Failed},
    text::{Language, Line, Page, Reader, Rect, Word},
};
use windows::{
    Globalization::Language as WinLanguage,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    core::HSTRING,
};

#[derive(Default)]
pub struct WindowsOcr {
    /// The last language's engine: a language switch makes a new one, a second read reuses it.
    engine: Option<(String, OcrEngine)>,
}

impl WindowsOcr {
    pub fn new() -> Self {
        Self::default()
    }

    fn engine(&mut self, tag: &str) -> Result<&OcrEngine, Failed> {
        if self.engine.as_ref().is_none_or(|(t, _)| t != tag) {
            let language = WinLanguage::CreateLanguage(&HSTRING::from(tag))?;
            if !OcrEngine::IsLanguageSupported(&language)? {
                return Err(Failed::because(Cause::Missing, tag));
            }
            self.engine = Some((tag.to_owned(), OcrEngine::TryCreateFromLanguage(&language)?));
        }
        Ok(&self.engine.as_ref().expect("set above").1)
    }
}

impl Reader for WindowsOcr {
    fn languages(&self) -> Vec<Language> {
        let Ok(list) = OcrEngine::AvailableRecognizerLanguages() else {
            return Vec::new();
        };
        let mut languages: Vec<Language> = list
            .into_iter()
            .filter_map(|l| {
                Some(Language {
                    tag: l.LanguageTag().ok()?.to_string(),
                    name: l.NativeName().ok()?.to_string(),
                })
            })
            .collect();
        // Windows lists them in no useful order; its profile engine is the first of the user's
        // languages that has a pack.
        if let Some(first) = OcrEngine::TryCreateFromUserProfileLanguages()
            .and_then(|e| e.RecognizerLanguage())
            .and_then(|l| l.LanguageTag())
            .ok()
            && let Some(i) = languages.iter().position(|l| first == *l.tag)
        {
            let preferred = languages.remove(i);
            languages.insert(0, preferred);
        }
        languages
    }

    fn read(&mut self, image: &Image, language: &str) -> Result<Page, Failed> {
        let max = OcrEngine::MaxImageDimension()?;
        if image.width > max || image.height > max {
            return Err(Failed::because(
                Cause::Unsupported,
                format!("{}×{}", image.width, image.height),
            ));
        }
        let bitmap = bitmap(image)?;
        let result = self.engine(language)?.RecognizeAsync(&bitmap)?.join()?;
        let mut lines = Vec::new();
        for line in result.Lines()? {
            let text = line.Text()?.to_string();
            let mut words = Vec::new();
            let mut at = 0;
            for word in line.Words()? {
                let word_text = word.Text()?.to_string();
                let r = word.BoundingRect()?;
                // The line's text has the spaces Windows put between words; a word found after
                // a space is spaced. Japanese and Chinese lines have none.
                let found = text[at..].find(&word_text).map(|i| at + i);
                let spaced = match found {
                    Some(i) => !words.is_empty() && text[at..i].contains(' '),
                    None => !words.is_empty(),
                };
                if let Some(i) = found {
                    at = i + word_text.len();
                }
                words.push(Word {
                    text: word_text,
                    rect: Rect {
                        x: r.X,
                        y: r.Y,
                        width: r.Width,
                        height: r.Height,
                    },
                    spaced,
                });
            }
            if !words.is_empty() {
                lines.push(Line { words });
            }
        }
        Ok(Page { lines })
    }
}

fn bitmap(image: &Image) -> windows::core::Result<SoftwareBitmap> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(&image.pixels)?;
    SoftwareBitmap::CreateCopyFromBuffer(
        &writer.DetachBuffer()?,
        BitmapPixelFormat::Bgra8,
        image.width as i32,
        image.height as i32,
    )
}
