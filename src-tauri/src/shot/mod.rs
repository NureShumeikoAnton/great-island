//! Screenshot: a screen capture whose fate is decided while its card is visible on the island.
//!
//! - [`Shot`] is a taken but undecided screenshot: when the card closes it is saved to the folder.
//!   Ctrl+C copies it to the clipboard (the file is saved too), Ctrl+X puts it only in the clipboard:
//!   no file, and if one is already written (the shot was opened in the viewer) it is deleted.
//!   From the viewer the shot can also be deleted;
//! - the ports [`Gallery`] (where to save) and [`Clipboard`] have their Windows implementations in
//!   `adapters/`, supplied by the island, so the logic is covered by tests.
//!
//! The picture, its time stamp and the status come from `capture`: a recording uses the same.

pub mod adapters;
pub mod edit;

use crate::{
    capture::{Image, Stamp, Status},
    fault::Failed,
    lang::tr,
};
use std::path::{Path, PathBuf};

pub fn file_stem(stamp: &Stamp) -> String {
    stamp.named(&tr!("file.screenshot"))
}

pub trait Gallery {
    fn save(&mut self, image: &Image, stamp: Stamp) -> Result<PathBuf, Failed>;
    /// Delete the saved file for good, bypassing the recycle bin.
    fn remove(&mut self, path: &Path) -> Result<(), Failed>;
    /// Overwrite an already saved file. If that fails, the previous file is intact.
    fn write(&mut self, path: &Path, image: &Image) -> Result<(), Failed>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saved {
    Written,
    /// The edits changed nothing (pixel for pixel): files untouched.
    Unchanged,
    Failed(Failed),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Same,
    /// A new file next to it: "… (2).png". From then on the shot is that one; the previous file is untouched.
    Copy,
}

pub trait Clipboard {
    fn put(&mut self, image: &Image) -> Result<(), Failed>;
}

pub struct Shot {
    image: Image,
    stamp: Stamp,
    status: Status,
    saved: Option<PathBuf>,
}

impl Shot {
    pub fn new(image: Image, stamp: Stamp) -> Self {
        Self {
            image,
            stamp,
            status: Status::Fresh,
            saved: None,
        }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    pub fn path(&self) -> Option<&Path> {
        self.saved.as_deref()
    }

    pub fn copy(&mut self, clipboard: &mut impl Clipboard) -> Result<(), Failed> {
        clipboard.put(&self.image)?;
        self.status = Status::Copied;
        Ok(())
    }

    /// Ctrl+X: only the clipboard. If the file is already written (the shot was opened) it is deleted;
    /// if deleting fails it stays, and this is just a copy.
    pub fn cut(
        &mut self,
        clipboard: &mut impl Clipboard,
        gallery: &mut impl Gallery,
    ) -> Result<(), Failed> {
        clipboard.put(&self.image)?;
        if let Some(path) = &self.saved
            && gallery.remove(path).is_err()
        {
            self.status = Status::Copied;
            return Ok(());
        }
        self.saved = None;
        self.status = Status::ClipboardOnly;
        Ok(())
    }

    /// Delete the shot: the written file is erased for good. If that fails (the file is held by another program)
    /// the shot comes back as it was.
    pub fn delete(self, gallery: &mut impl Gallery) -> Result<(), (Self, Failed)> {
        match self.saved.as_deref().map(|path| gallery.remove(path)) {
            Some(Err(failed)) => Err((self, failed)),
            _ => Ok(()),
        }
    }

    /// Save now, if not saved yet and not decided to go without a file.
    /// Returns the file path; `None` = no file needed.
    pub fn save(&mut self, gallery: &mut impl Gallery) -> Result<Option<&Path>, Failed> {
        if self.status == Status::ClipboardOnly {
            return Ok(None);
        }
        if self.saved.is_none() {
            self.saved = Some(gallery.save(&self.image, self.stamp)?);
        }
        Ok(self.saved.as_deref())
    }

    /// Saves the edited shot. Saving is an explicit action, so a file appears even if the shot was earlier kept
    /// only in the clipboard. If the edits changed nothing: no overwrite and no copy; the pixels themselves are
    /// compared (exact, and no slower than a hash, which would have to read every pixel anyway).
    pub fn save_edited(&mut self, image: Image, gallery: &mut impl Gallery, to: Target) -> Saved {
        if image == self.image {
            return Saved::Unchanged;
        }
        let path = match (&self.saved, to) {
            (Some(path), Target::Same) => gallery.write(path, &image).map(|()| path.clone()),
            _ => gallery.save(&image, self.stamp),
        };
        let path = match path {
            Ok(path) => path,
            Err(failed) => return Saved::Failed(failed),
        };
        self.saved = Some(path);
        self.image = image;
        if self.status == Status::ClipboardOnly {
            self.status = Status::Fresh;
        }
        Saved::Written
    }

    pub fn finish(mut self, gallery: &mut impl Gallery) -> Result<Option<PathBuf>, Failed> {
        self.save(gallery)?;
        Ok(self.saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::Cause;

    #[derive(Default)]
    struct Album {
        saves: usize,
        removed: usize,
        writes: usize,
        locked: bool,
        full: bool,
    }

    impl Gallery for Album {
        fn save(&mut self, _image: &Image, stamp: Stamp) -> Result<PathBuf, Failed> {
            if self.full {
                return Err(Failed::new(Cause::DiskFull));
            }
            self.saves += 1;
            let n = if self.saves == 1 {
                String::new()
            } else {
                format!(" ({})", self.saves)
            };
            Ok(PathBuf::from(format!("{}{n}.png", file_stem(&stamp))))
        }

        fn remove(&mut self, _path: &Path) -> Result<(), Failed> {
            self.removed += usize::from(!self.locked);
            self.unlocked()
        }

        fn write(&mut self, _path: &Path, _image: &Image) -> Result<(), Failed> {
            self.writes += usize::from(!self.locked);
            self.unlocked()
        }
    }

    impl Album {
        fn unlocked(&self) -> Result<(), Failed> {
            if self.locked {
                Err(Failed::new(Cause::Busy))
            } else {
                Ok(())
            }
        }
    }

    struct Board {
        free: bool,
        puts: usize,
    }

    impl Clipboard for Board {
        fn put(&mut self, _image: &Image) -> Result<(), Failed> {
            self.puts += usize::from(self.free);
            if self.free {
                Ok(())
            } else {
                Err(Failed::new(Cause::Busy))
            }
        }
    }

    fn board() -> Board {
        Board {
            free: true,
            puts: 0,
        }
    }

    fn image(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            pixels: vec![255; (width * height * 4) as usize],
        }
    }

    fn shot() -> Shot {
        Shot::new(image(4, 2), Stamp::default())
    }

    #[test]
    fn name_follows_windows() {
        let stamp = Stamp {
            year: 2026,
            month: 11,
            day: 1,
            hour: 9,
            minute: 5,
            second: 7,
        };
        crate::lang::with(crate::lang::Lang::Ru, || {
            assert_eq!(file_stem(&stamp), "Снимок экрана 2026-11-01 090507");
        });
        crate::lang::with(crate::lang::Lang::En, || {
            assert_eq!(file_stem(&stamp), "Screenshot 2026-11-01 090507");
        });
    }

    #[test]
    fn untouched_shot_is_saved_once_when_the_card_closes() {
        let mut album = Album::default();
        let path = shot().finish(&mut album);
        assert!(matches!(path, Ok(Some(_))));
        assert_eq!(album.saves, 1);
    }

    #[test]
    fn copy_keeps_the_file() {
        let (mut album, mut board) = (Album::default(), board());
        let mut shot = shot();
        assert!(shot.copy(&mut board).is_ok());
        assert_eq!(shot.status(), Status::Copied);
        assert!(matches!(shot.finish(&mut album), Ok(Some(_))));
        assert_eq!((board.puts, album.saves), (1, 1));
    }

    #[test]
    fn cut_leaves_only_the_clipboard() {
        let (mut album, mut board) = (Album::default(), board());
        let mut shot = shot();
        assert!(shot.cut(&mut board, &mut album).is_ok());
        assert_eq!(shot.status(), Status::ClipboardOnly);
        assert_eq!(shot.finish(&mut album), Ok(None));
        assert_eq!((album.saves, album.removed), (0, 0));
    }

    #[test]
    fn copy_after_cut_brings_the_file_back() {
        let (mut album, mut board) = (Album::default(), board());
        let mut shot = shot();
        let _ = shot.cut(&mut board, &mut album);
        let _ = shot.copy(&mut board);
        assert!(matches!(shot.finish(&mut album), Ok(Some(_))));
    }

    #[test]
    fn busy_clipboard_changes_nothing() {
        let mut busy = Board {
            free: false,
            puts: 0,
        };
        let mut shot = shot();
        let refused = shot.cut(&mut busy, &mut Album::default());
        assert_eq!(refused.map_err(|f| f.cause), Err(Cause::Busy));
        assert_eq!(shot.status(), Status::Fresh);
    }

    #[test]
    fn full_disk_is_told_and_the_shot_stays_unsaved() {
        let mut album = Album {
            full: true,
            ..Album::default()
        };
        let mut shot = shot();
        assert_eq!(
            shot.save(&mut album).map_err(|f| f.cause),
            Err(Cause::DiskFull)
        );
        assert!(shot.path().is_none());
        album.full = false;
        assert!(matches!(shot.finish(&mut album), Ok(Some(_))));
    }

    #[test]
    fn opened_shot_is_not_saved_twice() {
        let mut album = Album::default();
        let mut shot = shot();
        assert!(matches!(shot.save(&mut album), Ok(Some(_))));
        assert!(matches!(shot.save(&mut album), Ok(Some(_))));
        assert!(matches!(shot.finish(&mut album), Ok(Some(_))));
        assert_eq!(album.saves, 1);
    }

    #[test]
    fn cut_after_opening_removes_the_file() {
        let (mut album, mut board) = (Album::default(), board());
        let mut shot = shot();
        let _ = shot.save(&mut album);
        assert!(shot.cut(&mut board, &mut album).is_ok());
        assert_eq!(shot.status(), Status::ClipboardOnly);
        assert!(shot.path().is_none());
        assert_eq!(shot.finish(&mut album), Ok(None));
        assert_eq!((album.saves, album.removed), (1, 1));
    }

    #[test]
    fn cut_keeps_a_locked_file_as_a_copy() {
        let (mut album, mut board) = (Album::default(), board());
        let mut shot = shot();
        let _ = shot.save(&mut album);
        album.locked = true;
        assert!(shot.cut(&mut board, &mut album).is_ok());
        assert_eq!(shot.status(), Status::Copied);
        assert!(shot.path().is_some());
    }

    #[test]
    fn delete_removes_the_file() {
        let mut album = Album::default();
        let mut shot = shot();
        let _ = shot.save(&mut album);
        assert!(shot.delete(&mut album).is_ok());
        assert_eq!((album.saves, album.removed), (1, 1));
    }

    #[test]
    fn locked_file_is_not_deleted_and_the_shot_comes_back() {
        let mut album = Album::default();
        let mut shot = shot();
        let _ = shot.save(&mut album);
        album.locked = true;
        let Err((back, failed)) = shot.delete(&mut album) else {
            panic!("the shot must come back");
        };
        assert!(back.path().is_some());
        assert_eq!(failed.cause, Cause::Busy);
    }

    #[test]
    fn edited_shot_overwrites_its_file() {
        let mut album = Album::default();
        let mut shot = shot();
        let first = shot.save(&mut album).ok().flatten().map(Path::to_path_buf);
        assert_eq!(
            shot.save_edited(image(2, 2), &mut album, Target::Same),
            Saved::Written
        );
        assert_eq!(shot.path().map(Path::to_path_buf), first);
        assert_eq!(shot.image().width, 2);
        assert_eq!((album.saves, album.writes), (1, 1));
    }

    #[test]
    fn edited_copy_becomes_the_shot_and_keeps_the_original() {
        let mut album = Album::default();
        let mut shot = shot();
        let first = shot.save(&mut album).ok().flatten().map(Path::to_path_buf);
        assert_eq!(
            shot.save_edited(image(2, 2), &mut album, Target::Copy),
            Saved::Written
        );
        assert_ne!(shot.path().map(Path::to_path_buf), first);
        assert_eq!((album.saves, album.writes, album.removed), (2, 0, 0));
    }

    #[test]
    fn locked_file_keeps_the_old_picture() {
        let mut album = Album::default();
        let mut shot = shot();
        let _ = shot.save(&mut album);
        album.locked = true;
        assert_eq!(
            shot.save_edited(image(2, 2), &mut album, Target::Same),
            Saved::Failed(Failed::new(Cause::Busy))
        );
        assert_eq!(shot.image().width, 4);
    }

    #[test]
    fn saving_an_edit_brings_the_file_back_after_cut() {
        let (mut album, mut board) = (Album::default(), board());
        let mut shot = shot();
        let _ = shot.cut(&mut board, &mut album);
        assert_eq!(
            shot.save_edited(image(2, 2), &mut album, Target::Same),
            Saved::Written
        );
        assert_eq!(shot.status(), Status::Fresh);
        assert!(shot.path().is_some());
    }

    #[test]
    fn unchanged_edit_writes_nothing() {
        let mut album = Album::default();
        let mut shot = shot();
        let _ = shot.save(&mut album);
        for to in [Target::Same, Target::Copy] {
            assert_eq!(
                shot.save_edited(image(4, 2), &mut album, to),
                Saved::Unchanged
            );
        }
        assert_eq!((album.saves, album.writes), (1, 0));
    }
}
