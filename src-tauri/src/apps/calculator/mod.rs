//! The calculator: the first app, and the one that proves the apps layer. It needs nothing from
//! the system; its page is the expression line, the number and a grid of keys, the keyboard types
//! into it (brackets too), and a click on the number (or Ctrl+C) copies it. Every opening starts
//! from zero ([`Lifetime::Page`]).

mod engine;

use super::{
    Applet, Context, Effect, Event, Icon, Key, Kind, Lifetime, Manifest,
    ui::{Action, Align, Node, Style, Symbol, Tone},
};
use crate::lang::{self, tr};
use engine::{Engine, Op, Problem, Separators};

const POINT: u16 = 10;
const ADD: u16 = 11;
const SUB: u16 = 12;
const MUL: u16 = 13;
const DIV: u16 = 14;
const EQUALS: u16 = 15;
const PERCENT: u16 = 16;
const NEGATE: u16 = 17;
const CLEAR: u16 = 18;
const BACK: u16 = 19;
const COPY: u16 = 20;
/// The keyboard's brackets: the page has no keys for them.
const OPEN: u16 = 21;
const CLOSE: u16 = 22;

#[derive(Default)]
pub struct Calculator {
    engine: Engine,
}

fn separators() -> Separators {
    let (decimal, group) = lang::current().number_separators();
    Separators {
        decimal: decimal.into(),
        group: group.into(),
    }
}

fn problem_text(problem: Problem) -> String {
    match problem {
        Problem::DivideByZero => tr!("app.calculator.divide_by_zero"),
        Problem::Undefined => tr!("app.calculator.undefined"),
        Problem::Overflow => tr!("app.calculator.overflow"),
    }
    .into()
}

impl Calculator {
    fn press(&mut self, code: u16) -> Vec<Effect> {
        let e = &mut self.engine;
        match code {
            0..=9 => e.digit(code as u8),
            POINT => e.point(),
            ADD => e.op(Op::Add),
            SUB => e.op(Op::Sub),
            MUL => e.op(Op::Mul),
            DIV => e.op(Op::Div),
            EQUALS => e.equals(),
            PERCENT => e.percent(),
            NEGATE => e.negate(),
            CLEAR => e.clear(),
            BACK => e.backspace(),
            OPEN => e.open(),
            CLOSE => e.close(),
            COPY => {
                return e
                    .copied(&separators())
                    .map(Effect::Copy)
                    .into_iter()
                    .collect();
            }
            _ => {}
        }
        Vec::new()
    }

    fn key_code(key: &Key) -> Option<u16> {
        Some(match key {
            Key::Char(c @ '0'..='9') => *c as u16 - '0' as u16,
            Key::Char('.' | ',') => POINT,
            Key::Char('+') => ADD,
            Key::Char('-' | '−') => SUB,
            Key::Char('*' | '×' | 'x' | 'X') => MUL,
            Key::Char('/' | '÷' | ':') => DIV,
            Key::Char('%') => PERCENT,
            Key::Char('(') => OPEN,
            Key::Char(')') => CLOSE,
            Key::Char('=') | Key::Enter => EQUALS,
            Key::Backspace => BACK,
            Key::Delete => CLEAR,
            Key::Copy => COPY,
            Key::Char(_) => return None,
        })
    }
}

impl Applet for Calculator {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "calculator",
            title: tr!("app.calculator.title").into(),
            short: String::new(),
            icon: Icon::Calculator,
            kind: Kind::App,
            keyboard: true,
            lifetime: Lifetime::Page,
            fields: Vec::new(),
        }
    }

    fn page(&self, _cx: &Context) -> Node {
        let sep = separators();
        let number = match self.engine.problem() {
            Some(problem) => problem_text(problem),
            None => self.engine.display(&sep),
        };
        let key = |label: &str, code: u16, tone: Tone| Node::button(label, Action(code), tone);
        let digit = |d: u16| key(&d.to_string(), d, Tone::Plain);
        Node::Column(vec![
            Node::tap(
                Action(COPY),
                Node::Column(vec![
                    Node::text(self.engine.expression(&sep), Style::Caption, Align::End),
                    Node::text(number, Style::Display, Align::End),
                ]),
            ),
            Node::Grid {
                columns: 4,
                cells: vec![
                    key("C", CLEAR, Tone::Muted),
                    key("±", NEGATE, Tone::Muted),
                    key("%", PERCENT, Tone::Muted),
                    key("÷", DIV, Tone::Plain),
                    digit(7),
                    digit(8),
                    digit(9),
                    key("×", MUL, Tone::Plain),
                    digit(4),
                    digit(5),
                    digit(6),
                    key("−", SUB, Tone::Plain),
                    digit(1),
                    digit(2),
                    digit(3),
                    key("+", ADD, Tone::Plain),
                    Node::symbol(Symbol::Backspace, Action(BACK), Tone::Muted),
                    digit(0),
                    key(&sep.decimal, POINT, Tone::Plain),
                    key("=", EQUALS, Tone::Accent),
                ],
            },
        ])
    }

    fn update(&mut self, event: Event, _cx: &Context) -> Vec<Effect> {
        match event {
            Event::Press(Action(code)) => self.press(code),
            Event::Key(key) => match Self::key_code(&key) {
                Some(code) => self.press(code),
                None => Vec::new(),
            },
            // Nothing runs in the background and nothing is kept: the rest needs no answer.
            Event::Started
            | Event::Opened
            | Event::Closed
            | Event::Timer(_)
            | Event::Fetched { .. }
            | Event::Values => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{prefs::Values, ui::Face};
    use crate::lang::{self, Lang};

    fn run(calc: &mut Calculator, events: Vec<Event>) -> Vec<Effect> {
        let values = Values::default();
        let cx = Context::bare(&values);
        events
            .into_iter()
            .flat_map(|e| calc.update(e, &cx))
            .collect()
    }

    fn shown(calc: &Calculator) -> (String, String) {
        let values = Values::default();
        let page = calc.page(&Context::bare(&values));
        let Node::Column(parts) = page else {
            panic!("a column")
        };
        let Node::Tap { child, .. } = &parts[0] else {
            panic!("the number first")
        };
        let Node::Column(lines) = child.as_ref() else {
            panic!("two lines")
        };
        let text = |n: &Node| match n {
            Node::Text { text, .. } => text.clone(),
            _ => panic!("a text"),
        };
        (text(&lines[0]), text(&lines[1]))
    }

    fn typed(text: &str) -> Vec<Event> {
        text.chars().map(|c| Event::Key(Key::Char(c))).collect()
    }

    #[test]
    fn typing_on_the_keyboard_counts() {
        lang::with(Lang::Ru, || {
            let mut calc = Calculator::default();
            run(&mut calc, typed("1234,5*2"));
            run(&mut calc, vec![Event::Key(Key::Enter)]);
            assert_eq!(
                shown(&calc),
                ("1\u{a0}234,5 × 2 =".into(), "2\u{a0}469".into())
            );
        });
    }

    #[test]
    fn the_keys_on_the_page_count() {
        let mut calc = Calculator::default();
        let presses = [7, MUL, 6, EQUALS].map(|c| Event::Press(Action(c)));
        run(&mut calc, presses.into());
        assert_eq!(shown(&calc).1, "42");
    }

    #[test]
    fn the_number_is_copied_as_other_programs_read_it() {
        lang::with(Lang::Ru, || {
            let mut calc = Calculator::default();
            run(&mut calc, typed("1000-1234,5="));
            let copied = run(&mut calc, vec![Event::Press(Action(COPY))]);
            assert_eq!(copied, [Effect::Copy("-234,5".into())]);
            assert_eq!(run(&mut calc, vec![Event::Key(Key::Copy)]), copied);
        });
    }

    #[test]
    fn a_problem_takes_the_number_s_place_and_copies_nothing() {
        lang::with(Lang::Ru, || {
            let mut calc = Calculator::default();
            run(&mut calc, typed("1/0="));
            assert_eq!(shown(&calc).1, "Деление на ноль невозможно");
            assert!(run(&mut calc, vec![Event::Key(Key::Copy)]).is_empty());
        });
    }

    #[test]
    fn the_point_key_is_the_language_s() {
        let point = |lang| {
            lang::with(lang, || {
                let values = Values::default();
                let page = Calculator::default().page(&Context::bare(&values));
                let Node::Column(parts) = page else { panic!() };
                let Node::Grid { cells, .. } = &parts[1] else {
                    panic!()
                };
                match &cells[18] {
                    Node::Button {
                        face: Face::Label(label),
                        ..
                    } => label.clone(),
                    _ => panic!(),
                }
            })
        };
        assert_eq!(point(Lang::En), ".");
        assert_eq!(point(Lang::Ru), ",");
    }
}
