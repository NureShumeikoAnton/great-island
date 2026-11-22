//! The Windows side of reading text: the engines behind [`crate::text::Reader`].

mod ocr;

pub use ocr::WindowsOcr;
