//! A file name a capture can still take.

use std::path::{Path, PathBuf};

/// "<stem>.<ext>" in `dir`; if taken, "<stem> (2).<ext>" and so on.
pub fn free_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .expect("a free name exists")
}
