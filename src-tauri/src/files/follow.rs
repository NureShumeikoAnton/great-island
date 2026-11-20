//! Space in Explorer, as in QuickLook: Space on a photo or a video opens it, and the window
//! then follows the selection there. Pure rules; what is selected and which window is in
//! front come from `adapters/explorer.rs`.
//!
//! - Space opens only what the viewer reads, and does nothing on the file already shown;
//! - the window follows the selection: a photo or a video in its window, anything else (a
//!   GIF, a document, a folder) as a small card with the file's sheet, name, size and date;
//! - nothing selected, the same file again, or the editor open: the window stays as it is.

use super::open::Window;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub folder: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Show {
    File(Window, PathBuf),
    Card(Item),
}

impl Show {
    pub fn path(&self) -> &PathBuf {
        match self {
            Self::File(_, path) => path,
            Self::Card(item) => &item.path,
        }
    }

    fn of(item: &Item) -> Self {
        match Window::of(&item.path.to_string_lossy()) {
            Some(window) if !item.folder => Self::File(window, item.path.clone()),
            _ => Self::Card(item.clone()),
        }
    }
}

#[derive(Default)]
pub struct Follow {
    /// What the window opened from Explorer shows; `None` = no such window.
    shown: Option<PathBuf>,
    /// The editor is open: the window doesn't change under it.
    held: bool,
}

impl Follow {
    pub fn space(&self, item: &Item) -> Option<Show> {
        if self.shown.as_ref() == Some(&item.path) {
            return None;
        }
        match Show::of(item) {
            Show::Card(_) => None,
            show => Some(show),
        }
    }

    /// The window opened from Explorer shows `path`; `None` = it closed, or it shows a file
    /// opened another way.
    pub fn showing(&mut self, path: Option<PathBuf>) {
        if path.is_none() {
            self.held = false;
        }
        self.shown = path;
    }

    pub fn following(&self) -> bool {
        self.shown.is_some()
    }

    pub fn hold(&mut self, held: bool) {
        self.held = held;
    }

    pub fn selected(&self, item: Option<&Item>) -> Option<Show> {
        let item = item?;
        let shown = self.shown.as_ref()?;
        if self.held || *shown == item.path {
            return None;
        }
        Some(Show::of(item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Item {
        Item {
            path: PathBuf::from(r"C:\Pictures").join(name),
            folder: false,
        }
    }

    #[test]
    fn space_opens_a_photo_or_a_video_and_nothing_else() {
        let follow = Follow::default();
        assert_eq!(
            follow.space(&file("a.jpg")),
            Some(Show::File(Window::Photo, file("a.jpg").path))
        );
        assert!(matches!(
            follow.space(&file("b.mp4")),
            Some(Show::File(Window::Video, _))
        ));
        assert_eq!(follow.space(&file("c.gif")), None);
        assert_eq!(follow.space(&file("d.docx")), None);
        let folder = Item {
            path: PathBuf::from(r"C:\Pictures\Trip.jpg"),
            folder: true,
        };
        assert_eq!(follow.space(&folder), None);
    }

    #[test]
    fn space_on_the_file_already_shown_does_nothing() {
        let mut follow = Follow::default();
        follow.showing(Some(file("a.jpg").path));
        assert_eq!(follow.space(&file("a.jpg")), None);
        assert!(follow.space(&file("b.jpg")).is_some());
    }

    #[test]
    fn the_window_follows_the_selection_onto_anything() {
        let mut follow = Follow::default();
        assert_eq!(follow.selected(Some(&file("b.jpg"))), None, "no window yet");
        follow.showing(Some(file("a.jpg").path));
        assert!(follow.following());
        assert_eq!(follow.selected(Some(&file("a.jpg"))), None);
        assert_eq!(follow.selected(None), None);
        assert!(matches!(
            follow.selected(Some(&file("b.mov"))),
            Some(Show::File(Window::Video, _))
        ));
        assert_eq!(
            follow.selected(Some(&file("c.gif"))),
            Some(Show::Card(file("c.gif")))
        );
    }

    #[test]
    fn the_editor_holds_the_window_until_it_closes() {
        let mut follow = Follow::default();
        follow.showing(Some(file("a.jpg").path));
        follow.hold(true);
        assert_eq!(follow.selected(Some(&file("b.jpg"))), None);
        follow.hold(false);
        assert!(follow.selected(Some(&file("b.jpg"))).is_some());
    }

    #[test]
    fn a_closed_window_follows_nothing() {
        let mut follow = Follow::default();
        follow.showing(Some(file("a.jpg").path));
        follow.hold(true);
        follow.showing(None);
        assert!(!follow.following());
        assert_eq!(follow.selected(Some(&file("b.jpg"))), None);
        follow.showing(Some(file("a.jpg").path));
        assert!(follow.selected(Some(&file("b.jpg"))).is_some(), "not held");
    }
}
