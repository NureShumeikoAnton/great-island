//! Dialog windows of the island (rules): what a dialog says ([`Sheet`]), where each part of it
//! stands ([`layout`]) and how its notes scroll ([`scroll`]).
//!
//! A feature describes its dialog as a sheet; the island's dialog window (`island::dialog`) draws
//! it and answers a press with the button's [`Action`]. Every dialog has one look: the app's icon,
//! a title with a line under it, then the blocks in order.

pub mod layout;
pub mod scroll;

/// What a button means to the dialog's owner; the dialog only hands it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Action(pub u16);

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// Small capitals over a group of items.
    Heading(String),
    Item(String),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub label: String,
    pub action: Action,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Notes {
        title: String,
        lines: Vec<Line>,
    },
    /// `share` from 0 to 1; `None` — under way, with no measure. The line under the bar holds
    /// `left` and `right`; both empty — no line.
    Progress {
        share: Option<f32>,
        left: String,
        right: String,
    },
    Buttons(Vec<Button>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub title: String,
    /// Empty — none.
    pub subtitle: String,
    pub blocks: Vec<Block>,
    /// The × in the corner and Esc; `None` — neither.
    pub dismiss: Option<Action>,
}

impl Sheet {
    /// `next` differs from this one only in how far its progress bars are and what their lines
    /// say: the window moves the bars instead of drawing the card again.
    pub fn moves_only(&self, next: &Sheet) -> bool {
        let frame = |s: &Sheet| Sheet {
            blocks: s
                .blocks
                .iter()
                .map(|b| match b {
                    Block::Progress { share, .. } => Block::Progress {
                        share: share.map(|_| 0.0),
                        left: String::new(),
                        right: String::new(),
                    },
                    other => other.clone(),
                })
                .collect(),
            ..s.clone()
        };
        frame(self) == frame(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downloading(share: Option<f32>, left: &str) -> Sheet {
        Sheet {
            title: "Downloading".into(),
            subtitle: String::new(),
            blocks: vec![Block::Progress {
                share,
                left: left.into(),
                right: String::new(),
            }],
            dismiss: None,
        }
    }

    #[test]
    fn only_the_progress_moving_keeps_the_card() {
        let a = downloading(Some(0.1), "1 of 9 MB");
        assert!(a.moves_only(&downloading(Some(0.5), "5 of 9 MB")));
        assert!(
            !a.moves_only(&downloading(None, "")),
            "a bar under way is another bar"
        );
        let mut retitled = downloading(Some(0.5), "");
        retitled.title = "Checking".into();
        assert!(!a.moves_only(&retitled));
    }
}
