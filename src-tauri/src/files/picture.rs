//! A photo opened from the disk into the viewer: what becomes of the file when the editor
//! saves or the viewer deletes. Unlike a screenshot, the file is the user's: an edit goes
//! into the file's own format (or a JPEG beside a format Windows can't write, `open.rs`), and
//! a deleted one goes to the Recycle Bin.
//!
//! The disk side is the [`Album`] port; its Windows implementation is in `adapters/photo.rs`.

use super::open::{Destination, Format, folder, stem};
use crate::{
    capture::Image,
    fault::Failed,
    shot::{Saved, Target},
};
use std::path::{Path, PathBuf};

pub trait Album {
    /// `like` is the file whose metadata (camera, date) the new one keeps. If writing fails,
    /// a file at `path` is intact.
    fn write(
        &mut self,
        path: &Path,
        image: &Image,
        format: Format,
        like: &Path,
    ) -> Result<(), Failed>;
    fn free_name(&self, dir: &Path, stem: &str, ext: &str) -> PathBuf;
    /// false = the user answered Windows' question by keeping the file (a file too large for
    /// the Recycle Bin is asked about).
    fn recycle(&mut self, path: &Path) -> Result<bool, Failed>;
}

pub enum Deleted {
    Gone,
    Kept(Picture),
    Failed(Picture, Failed),
}

pub struct Picture {
    path: PathBuf,
    format: Format,
    image: Image,
}

impl Picture {
    /// `None` for a name the viewer doesn't read.
    pub fn new(path: PathBuf, image: Image) -> Option<Self> {
        let format = Format::of(&path.to_string_lossy())?;
        Some(Self {
            path,
            format,
            image,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The edits changed nothing (pixel for pixel): no file is touched.
    pub fn save_edited(&mut self, image: Image, album: &mut impl Album, to: Target) -> Saved {
        if image == self.image {
            return Saved::Unchanged;
        }
        let (path, format) = match self.format.edited(to == Target::Copy) {
            Destination::Replace(format) => (self.path.clone(), format),
            Destination::Beside(format) => (
                album.free_name(&folder(&self.path), &stem(&self.path), format.extension()),
                format,
            ),
        };
        if let Err(failed) = album.write(&path, &image, format, &self.path) {
            return Saved::Failed(failed);
        }
        self.path = path;
        self.format = format;
        self.image = image;
        Saved::Written
    }

    /// To the Recycle Bin. If that fails (the file is held by another program) the picture
    /// comes back as it was.
    pub fn delete(self, album: &mut impl Album) -> Deleted {
        match album.recycle(&self.path) {
            Ok(true) => Deleted::Gone,
            Ok(false) => Deleted::Kept(self),
            Err(failed) => Deleted::Failed(self, failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::Cause;
    use std::collections::HashSet;

    #[derive(Default)]
    struct Disk {
        files: HashSet<PathBuf>,
        written: Vec<(PathBuf, Format)>,
        recycled: Vec<PathBuf>,
        locked: bool,
        keep: bool,
    }

    impl Album for Disk {
        fn write(
            &mut self,
            path: &Path,
            _image: &Image,
            format: Format,
            _like: &Path,
        ) -> Result<(), Failed> {
            if self.locked {
                return Err(Failed::new(Cause::Busy));
            }
            self.files.insert(path.to_path_buf());
            self.written.push((path.to_path_buf(), format));
            Ok(())
        }

        fn free_name(&self, dir: &Path, stem: &str, ext: &str) -> PathBuf {
            (1..)
                .map(|n| {
                    let name = if n == 1 {
                        format!("{stem}.{ext}")
                    } else {
                        format!("{stem} ({n}).{ext}")
                    };
                    dir.join(name)
                })
                .find(|p| !self.files.contains(p))
                .expect("a free name")
        }

        fn recycle(&mut self, path: &Path) -> Result<bool, Failed> {
            if self.locked {
                return Err(Failed::new(Cause::Busy));
            }
            if self.keep {
                return Ok(false);
            }
            self.files.remove(path);
            self.recycled.push(path.to_path_buf());
            Ok(true)
        }
    }

    fn image(width: u32) -> Image {
        Image {
            width,
            height: 1,
            pixels: vec![255; (width * 4) as usize],
        }
    }

    fn opened(name: &str, disk: &mut Disk) -> Picture {
        let path = PathBuf::from(r"C:\Photos").join(name);
        disk.files.insert(path.clone());
        Picture::new(path, image(4)).expect("a photo")
    }

    #[test]
    fn a_jpeg_is_saved_over_itself_as_a_jpeg() {
        let mut disk = Disk::default();
        let mut photo = opened("IMG_1.jpg", &mut disk);
        assert_eq!(
            photo.save_edited(image(2), &mut disk, Target::Same),
            Saved::Written
        );
        assert_eq!(
            disk.written,
            [(PathBuf::from(r"C:\Photos\IMG_1.jpg"), Format::Jpeg)]
        );
        assert_eq!(photo.image().width, 2);
    }

    #[test]
    fn a_copy_takes_a_free_name_in_the_same_format() {
        let mut disk = Disk::default();
        let mut photo = opened("shot.png", &mut disk);
        let _ = photo.save_edited(image(2), &mut disk, Target::Copy);
        assert_eq!(photo.path(), Path::new(r"C:\Photos\shot (2).png"));
        assert_eq!(photo.name(), "shot (2).png");
        assert!(disk.files.contains(Path::new(r"C:\Photos\shot.png")));
    }

    #[test]
    fn a_heic_is_never_overwritten_and_its_edit_is_a_jpeg_beside_it() {
        let mut disk = Disk::default();
        let mut photo = opened("IMG_2.HEIC", &mut disk);
        let _ = photo.save_edited(image(2), &mut disk, Target::Same);
        assert_eq!(photo.path(), Path::new(r"C:\Photos\IMG_2.jpg"));
        assert_eq!(disk.written[0].1, Format::Jpeg);
        // Saved again, the JPEG is its own file now and is replaced.
        let _ = photo.save_edited(image(3), &mut disk, Target::Same);
        assert_eq!(photo.path(), Path::new(r"C:\Photos\IMG_2.jpg"));
        assert_eq!(disk.written.len(), 2);
    }

    #[test]
    fn an_edit_that_changed_nothing_writes_nothing() {
        let mut disk = Disk::default();
        let mut photo = opened("a.jpg", &mut disk);
        for to in [Target::Same, Target::Copy] {
            assert_eq!(photo.save_edited(image(4), &mut disk, to), Saved::Unchanged);
        }
        assert!(disk.written.is_empty());
    }

    #[test]
    fn a_locked_file_keeps_the_old_picture_and_comes_back_from_delete() {
        let mut disk = Disk::default();
        let mut photo = opened("a.jpg", &mut disk);
        disk.locked = true;
        assert_eq!(
            photo.save_edited(image(2), &mut disk, Target::Same),
            Saved::Failed(Failed::new(Cause::Busy))
        );
        assert_eq!(photo.image().width, 4);
        let Deleted::Failed(back, failed) = photo.delete(&mut disk) else {
            panic!("the photo must come back");
        };
        assert_eq!(failed.cause, Cause::Busy);
        assert_eq!(back.path(), Path::new(r"C:\Photos\a.jpg"));
    }

    #[test]
    fn delete_sends_the_file_to_the_recycle_bin() {
        let mut disk = Disk::default();
        let photo = opened("a.jpg", &mut disk);
        assert!(matches!(photo.delete(&mut disk), Deleted::Gone));
        assert_eq!(disk.recycled, [PathBuf::from(r"C:\Photos\a.jpg")]);
    }

    #[test]
    fn a_file_the_user_kept_stays_open() {
        let mut disk = Disk {
            keep: true,
            ..Disk::default()
        };
        let photo = opened("a.jpg", &mut disk);
        assert!(matches!(photo.delete(&mut disk), Deleted::Kept(_)));
        assert!(disk.recycled.is_empty());
    }
}
