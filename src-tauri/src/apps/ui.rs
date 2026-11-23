//! An app's page as data: a tree of a few kinds of parts that the island lays out and draws in
//! its own look (`island/view/app.rs`). An app says what is there, never where in pixels or in
//! what color: the island's look stays one for every app.

/// What a press means to the app; the island only hands it back in [`super::Event::Press`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Action(pub u16);

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Column(Vec<Node>),
    Row(Vec<Node>),
    /// Equal cells, filled row by row.
    Grid {
        columns: u8,
        cells: Vec<Node>,
    },
    Text {
        text: String,
        style: Style,
        align: Align,
    },
    Button {
        face: Face,
        action: Action,
        tone: Tone,
    },
    /// The child takes presses as a whole, e.g. a result copied by a click.
    Tap {
        action: Action,
        child: Box<Node>,
    },
    /// From 0 to 1.
    Progress(f32),
    /// An empty share of its row, or a gap in a column.
    Spacer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Title,
    Body,
    /// Grey, smaller: a hint, a unit, the expression over a result.
    Caption,
    /// A large number: a result, a time.
    Display,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Face {
    Label(String),
    Symbol(Symbol),
}

/// A sign no font draws well enough: the island draws it as one of its own icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Backspace,
    Siren,
    Warning,
    Check,
}

/// The color of news by what it means; the island picks the shades.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Alarm,
    Caution,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// A secondary key: clear, sign, percent.
    Muted,
    /// The one key that completes: equals, start.
    Accent,
}

impl Node {
    pub fn text(text: impl Into<String>, style: Style, align: Align) -> Self {
        Self::Text {
            text: text.into(),
            style,
            align,
        }
    }

    pub fn button(label: impl Into<String>, action: Action, tone: Tone) -> Self {
        Self::Button {
            face: Face::Label(label.into()),
            action,
            tone,
        }
    }

    pub fn symbol(symbol: Symbol, action: Action, tone: Tone) -> Self {
        Self::Button {
            face: Face::Symbol(symbol),
            action,
            tone,
        }
    }

    pub fn tap(action: Action, child: Node) -> Self {
        Self::Tap {
            action,
            child: Box::new(child),
        }
    }

    pub fn actions(&self) -> Vec<Action> {
        let mut found = Vec::new();
        self.collect(&mut found);
        found
    }

    fn collect(&self, found: &mut Vec<Action>) {
        match self {
            Self::Column(children)
            | Self::Row(children)
            | Self::Grid {
                cells: children, ..
            } => children.iter().for_each(|c| c.collect(found)),
            Self::Button { action, .. } => found.push(*action),
            Self::Tap { action, child } => {
                found.push(*action);
                child.collect(found);
            }
            Self::Text { .. } | Self::Progress(_) | Self::Spacer => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_found_in_the_tree_s_order() {
        let tree = Node::Column(vec![
            Node::tap(Action(9), Node::text("12", Style::Display, Align::End)),
            Node::Grid {
                columns: 2,
                cells: vec![
                    Node::button("1", Action(1), Tone::Plain),
                    Node::button("=", Action(2), Tone::Accent),
                ],
            },
        ]);
        assert_eq!(tree.actions(), [Action(9), Action(1), Action(2)]);
    }
}
