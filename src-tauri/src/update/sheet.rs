//! What the update window says (`crate::dialog`): the release's notes and the choices, then the
//! download, then a failure if it comes. Built again from the [`Status`] at each of its changes;
//! a status with nothing to show closes the window.

use super::{Phase, Status};
use crate::{
    dialog::{Action, Block, Button, Line, Sheet},
    lang::{self, tr},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Install = 1,
    Postpone = 2,
    Skip = 3,
    Close = 4,
}

impl Answer {
    const ALL: [Self; 4] = [Self::Install, Self::Postpone, Self::Skip, Self::Close];

    pub fn action(self) -> Action {
        Action(self as u16)
    }

    pub fn of(action: Action) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.action() == action)
    }
}

fn button(label: impl Into<String>, answer: Answer, primary: bool) -> Button {
    Button {
        label: label.into(),
        action: answer.action(),
        primary,
    }
}

/// `None`: nothing to show.
pub fn sheet(status: &Status) -> Option<Sheet> {
    let version = status
        .offer
        .as_ref()
        .map(|o| o.latest.to_string())
        .unwrap_or_default();
    let running = Block::Progress {
        share: None,
        left: String::new(),
        right: String::new(),
    };
    let sheet = match status.phase {
        Phase::Available => {
            let offer = status.offer.as_ref()?;
            let mut blocks = vec![
                Block::Notes {
                    title: tr!("update.notes").into_owned(),
                    lines: note_lines(offer.notes.pick(lang::current().code())),
                },
                Block::Buttons(vec![button(tr!("update.install"), Answer::Install, true)]),
            ];
            let mut others = Vec::new();
            if offer.can_postpone {
                others.push(button(tr!("update.postpone"), Answer::Postpone, false));
            }
            if offer.can_skip {
                others.push(button(tr!("update.skip"), Answer::Skip, false));
            }
            if !others.is_empty() {
                blocks.push(Block::Buttons(others));
            }
            Sheet {
                title: tr!("update.title").into_owned(),
                subtitle: tr!(
                    "update.versions",
                    latest = offer.latest,
                    current = offer.current
                )
                .into_owned(),
                blocks,
                dismiss: offer.can_postpone.then_some(Answer::Postpone.action()),
            }
        }
        Phase::Downloading => {
            let bar = if status.total > 0 {
                let share = (status.done as f32 / status.total as f32).min(1.0);
                Block::Progress {
                    share: Some(share),
                    left: tr!(
                        "update.size",
                        done = megabytes(status.done),
                        total = megabytes(status.total)
                    )
                    .into_owned(),
                    right: format!("{} %", (share * 100.0).round()),
                }
            } else {
                running
            };
            Sheet {
                title: tr!("update.downloading", version = version).into_owned(),
                subtitle: String::new(),
                blocks: vec![bar],
                dismiss: None,
            }
        }
        Phase::Verifying => Sheet {
            title: tr!("update.verifying").into_owned(),
            subtitle: String::new(),
            blocks: vec![running],
            dismiss: None,
        },
        Phase::Installing => Sheet {
            title: tr!("update.installing", version = version).into_owned(),
            subtitle: tr!("update.installing_hint").into_owned(),
            blocks: vec![running],
            dismiss: None,
        },
        Phase::InstallFailed => Sheet {
            title: tr!("update.failed").into_owned(),
            subtitle: status.error.clone().unwrap_or_default(),
            blocks: vec![Block::Buttons(vec![button(
                tr!("update.close"),
                Answer::Close,
                true,
            )])],
            dismiss: None,
        },
        Phase::Idle | Phase::Checking | Phase::UpToDate | Phase::Failed => return None,
    };
    Some(sheet)
}

/// One decimal, none when it is zero: "3.4", "12".
fn megabytes(bytes: u64) -> String {
    let mb = format!("{:.1}", bytes as f64 / (1024.0 * 1024.0));
    mb.strip_suffix(".0").map(str::to_string).unwrap_or(mb)
}

/// The notes are the changelog's text: "### Heading" and "- item" lines are all it uses.
pub fn note_lines(text: &str) -> Vec<Line> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            if let Some(heading) = line.strip_prefix("### ") {
                Line::Heading(heading.to_string())
            } else if let Some(item) = line.strip_prefix("- ") {
                Line::Item(item.replace('`', ""))
            } else {
                Line::Text(line.replace('`', ""))
            }
        })
        .collect()
}

/// The newest release's part of a changelog (the first "## " section with "### " groups), as the
/// manifest's notes carry it: for the demo of the window.
pub fn newest_notes(changelog: &str) -> String {
    changelog
        .split("\n## ")
        .skip(1)
        .map(|section| section.split_once('\n').map_or("", |(_, body)| body).trim())
        .find(|body| body.lines().any(|line| line.starts_with("### ")))
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lang::Lang,
        update::{Notes, OfferView},
    };

    fn offer(can_postpone: bool, can_skip: bool) -> OfferView {
        OfferView {
            current: "0.8.2".parse().expect("a version"),
            latest: "0.8.3".parse().expect("a version"),
            notes: Notes {
                en: "### Added\n- One".into(),
                ru: "### Добавлено\n\n- Кнопка `микрофона`\nПросто строка".into(),
            },
            can_skip,
            can_postpone,
            skipped: false,
            size: 0,
        }
    }

    fn status(phase: Phase) -> Status {
        Status {
            phase,
            offer: Some(offer(true, true)),
            ..Status::default()
        }
    }

    fn buttons(sheet: &Sheet) -> Vec<Vec<Answer>> {
        sheet
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Buttons(row) => Some(
                    row.iter()
                        .map(|b| Answer::of(b.action).expect("an answer"))
                        .collect(),
                ),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_offer_shows_the_notes_and_the_choices_it_allows() {
        lang::with(Lang::Ru, || {
            let s = sheet(&status(Phase::Available)).expect("a sheet");
            assert_eq!(s.title, "Доступно обновление");
            assert_eq!(s.subtitle, "Great Island 0.8.3 · у вас 0.8.2");
            assert_eq!(
                buttons(&s),
                [vec![Answer::Install], vec![Answer::Postpone, Answer::Skip]]
            );
            assert_eq!(s.dismiss, Some(Answer::Postpone.action()));
            let Block::Notes { title, lines } = &s.blocks[0] else {
                panic!("the notes first");
            };
            assert_eq!(title, "Что нового");
            assert_eq!(
                lines,
                &[
                    Line::Heading("Добавлено".into()),
                    Line::Item("Кнопка микрофона".into()),
                    Line::Text("Просто строка".into()),
                ]
            );
        });
    }

    #[test]
    fn an_offer_that_must_be_taken_has_only_install_and_no_dismiss() {
        let mut st = status(Phase::Available);
        st.offer = Some(offer(false, false));
        let s = sheet(&st).expect("a sheet");
        assert_eq!(buttons(&s), [vec![Answer::Install]]);
        assert_eq!(s.dismiss, None);
    }

    #[test]
    fn the_download_shows_its_share_or_runs_without_one() {
        lang::with(Lang::Ru, || {
            let mut st = status(Phase::Downloading);
            st.done = 3 * 1024 * 1024 + 400 * 1024;
            st.total = 12 * 1024 * 1024;
            let s = sheet(&st).expect("a sheet");
            assert_eq!(s.title, "Скачиваю 0.8.3…");
            let Block::Progress { share, left, right } = &s.blocks[0] else {
                panic!("a bar");
            };
            assert!((share.expect("a share") - 0.2832).abs() < 0.001);
            assert_eq!(left, "3.4 из 12 МБ");
            assert_eq!(right, "28 %");
            st.total = 0;
            let s = sheet(&st).expect("a sheet");
            assert!(
                matches!(&s.blocks[0], Block::Progress { share: None, left, .. } if left.is_empty())
            );
        });
    }

    #[test]
    fn a_failure_says_why_and_offers_to_close() {
        let mut st = status(Phase::InstallFailed);
        st.error = Some("access denied".into());
        let s = sheet(&st).expect("a sheet");
        assert_eq!(s.subtitle, "access denied");
        assert_eq!(buttons(&s), [vec![Answer::Close]]);
    }

    #[test]
    fn the_newest_release_with_changes_is_taken_from_the_changelog() {
        let changelog = "# Changes\n\nIntro\n\n## Unreleased\n\n## 0.8.2 — 2026-11-19\n\n### Added\n\n- One\n\n## 0.8.1\n\n### Fixed\n- Two\n";
        assert_eq!(newest_notes(changelog), "### Added\n\n- One");
        assert_eq!(newest_notes("nothing"), "");
    }

    #[test]
    fn nothing_to_show_closes_the_window() {
        for phase in [Phase::Idle, Phase::Checking, Phase::UpToDate, Phase::Failed] {
            assert_eq!(sheet(&status(phase)), None, "{phase:?}");
        }
        let mut st = status(Phase::Available);
        st.offer = None;
        assert_eq!(sheet(&st), None);
    }
}
