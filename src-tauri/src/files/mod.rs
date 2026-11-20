//! Files as the island shows them, whoever brought them (a download, a file opened from
//! Explorer): what kind a file is by its name, for the placeholder drawn in place of a picture
//! it doesn't have. Pure rules.
//!
//! - `open.rs`: a file opened from outside: which viewer window takes it, the photo formats,
//!   where an edit goes; `picture.rs`: such a photo in the viewer (save, delete);
//!   `follow.rs`: Space in Explorer and the window following the selection there;
//! - `adapters/`: the shell side every feature shares (open a file, show files in Explorer,
//!   Windows' known folders), photos through WIC, the Recycle Bin, the "Open with"
//!   registration, Explorer's selection.

pub mod adapters;
pub mod follow;
pub mod open;
pub mod picture;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    Document,
    Sheet,
    Slides,
    Text,
    Archive,
    App,
    Other,
}

const KINDS: [(Kind, &[&str]); 10] = [
    (
        Kind::Image,
        &[
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "heif", "avif", "svg", "ico",
            "tif", "tiff", "raw",
        ],
    ),
    (
        Kind::Video,
        &["mp4", "mkv", "mov", "avi", "webm", "wmv", "m4v", "flv"],
    ),
    (
        Kind::Audio,
        &["mp3", "wav", "flac", "m4a", "aac", "ogg", "opus", "wma"],
    ),
    (Kind::Pdf, &["pdf"]),
    (Kind::Document, &["doc", "docx", "odt", "rtf", "pages"]),
    (Kind::Sheet, &["xls", "xlsx", "ods", "csv", "numbers"]),
    (Kind::Slides, &["ppt", "pptx", "odp", "key"]),
    (
        Kind::Text,
        &[
            "txt", "md", "json", "xml", "html", "htm", "log", "ini", "yaml", "yml",
        ],
    ),
    (
        Kind::Archive,
        &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso", "cab"],
    ),
    (
        Kind::App,
        &["exe", "msi", "msix", "appx", "bat", "cmd", "ps1", "apk"],
    ),
];

impl Kind {
    pub fn of(name: &str) -> Self {
        let Some(ext) = extension(name) else {
            return Self::Other;
        };
        KINDS
            .iter()
            .find(|(_, list)| list.iter().any(|e| e.eq_ignore_ascii_case(ext)))
            .map_or(Self::Other, |(kind, _)| *kind)
    }
}

/// Without the dot; `None` for a name with no extension.
pub fn extension(name: &str) -> Option<&str> {
    name.rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
        .map(|(_, ext)| ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_told_by_the_extension_in_any_case() {
        assert_eq!(Kind::of("photo.JPG"), Kind::Image);
        assert_eq!(Kind::of("Лекция 7. Сети.pdf"), Kind::Pdf);
        assert_eq!(Kind::of("report.final.docx"), Kind::Document);
        assert_eq!(Kind::of("setup_x64.exe"), Kind::App);
        assert_eq!(Kind::of("backup.tar.gz"), Kind::Archive);
        assert_eq!(Kind::of("README"), Kind::Other);
        assert_eq!(Kind::of("model.bin"), Kind::Other);
    }

    #[test]
    fn a_dot_file_and_a_trailing_dot_have_no_extension() {
        assert_eq!(extension(".gitignore"), None);
        assert_eq!(extension("name."), None);
        assert_eq!(extension("a.b.c"), Some("c"));
    }
}
