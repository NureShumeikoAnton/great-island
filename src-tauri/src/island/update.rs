//! Updates on the island. The island owns the whole thing: it checks, it keeps the state
//! ([`Status`], written to `update-status.json` at every change), it downloads and installs. Everything
//! else only shows that state: the island's own notices (a spinner, a check mark, cards, problems), the
//! update window (a dialog, `dialogs.rs`, saying what `update::sheet` makes of the state) and "About" in
//! the settings (`settings_app.rs`), which sends commands ([`Command`]). A check the user asked for and a
//! check by the clock therefore look the same everywhere.
//!
//! A check by the clock is due at start and then once a day. The one at start runs as soon as nothing
//! important goes on; the daily one also waits for a minute without input (`update::calm`). Every check
//! shows a spinner and ends with an answer: a release found opens the update window at once (if something
//! important started meanwhile, in the next calm moment), a skipped or postponed one gets a notice. A press
//! of "check" during the clock's check makes it the user's. The window answers install, postpone or skip;
//! after every change of the state it shows the new state, and goes when there is nothing to show.
//! Installing: download on a thread, signature, then the installer is run silently and the island leaves;
//! the installer puts the new version in place and starts the island again.
//!
//! `--demo-update` opens the window with a sample release; its Install pretends to download and fails.
//!
//! Rules: `crate::update`. The network, the files and the processes: `crate::update::adapters`.

use super::{App, dialogs::Owner, view::UpdateLook, window::WM_UPDATE};
use crate::{
    dialog::Action,
    fault::{Cause, Failed, Fault, What},
    journal,
    lang::tr,
    persist, platform,
    stage::NoticeKind,
    update::{
        self, Command, Manifest, Memory, Notes, OfferView, Phase, Status, Verdict, Version,
        adapters::{self, http, process},
        sheet::{self, Answer as Reply, newest_notes},
        signature,
    },
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

/// After a failed check the next one waits this long, not for a day and not for the next tick.
const RETRY_MS: u64 = 60 * 60 * 1000;
/// The spinner turns at least once, even when the answer comes at once: a flash is not a status.
const MIN_SPIN_MS: u64 = 900;
const WRITE_EVERY_MS: u64 = 250;
const DEMO_SIZE: u64 = 24 * 1024 * 1024;

type Answer = Result<Manifest, Failed>;

enum Step {
    Bytes { done: u64, total: u64 },
    Ready(PathBuf),
    Failed(Failed),
}

pub(super) struct Updates {
    memory: Memory,
    /// The newest release found and not yet installed; also in `update-offer.json`.
    offer: Option<Manifest>,
    status: Status,
    check: Option<Receiver<Answer>>,
    /// Island ms when the spinner came up.
    spinning_since: u64,
    /// An answer that came before the spinner had turned: held, and whether the user asked for the check.
    answered: Option<(Answer, bool)>,
    /// A check was started in this run: until then the clock's check doesn't wait for its day.
    checked: bool,
    install: Option<Receiver<Step>>,
    /// `--demo-update`: the offer is a sample and its download pretends.
    demo: bool,
    /// What the update notice says: its look, what happened, what to do.
    notice: Option<(UpdateLook, String, String)>,
    /// Unix ms: no new check and no new window before it.
    retry_at: u64,
    /// A failed check by the clock goes to the journal once, until one succeeds.
    said: bool,
    /// Island ms of the last write of the status during a download.
    written: u64,
}

impl Updates {
    pub(super) fn new() -> Self {
        let this = Self {
            memory: adapters::load_memory(),
            offer: adapters::load_offer(),
            status: Status::default(),
            check: None,
            spinning_since: 0,
            answered: None,
            checked: false,
            install: None,
            demo: false,
            notice: None,
            retry_at: 0,
            said: false,
            written: 0,
        };
        // A file left by the last run says nothing about this one.
        let _ = adapters::save_status(&this.status);
        this
    }

    fn working(&self) -> bool {
        self.check.is_some() || self.answered.is_some() || self.install.is_some()
    }
}

fn current() -> Version {
    env!("CARGO_PKG_VERSION")
        .parse()
        .expect("the package version is x.y.z")
}

impl App {
    pub(super) fn update_notice(&self) -> Option<(UpdateLook, String, String)> {
        self.updates.notice.clone()
    }

    /// Once a second.
    pub(super) fn tick_updates(&mut self) {
        self.take_check();
        self.settle_answer();
        self.drain_install();
        if self.updates.working() || self.has_dialog(Owner::Update) || self.updates.demo {
            return;
        }
        if !self.settings.check_updates || self.update_busy() {
            return;
        }
        let now = persist::now_ms();
        // After a failed check, or an update window that went away without an answer: not at once again.
        if now < self.updates.retry_at {
            return;
        }
        // The first check of a run skips the quiet minute; a window or a daily check waits for it.
        let calm = update::calm(platform::idle_ms(), false);
        if !self.updates.checked {
            return self.start_check(false);
        }
        if !calm {
            return;
        }
        if let Some(offer) = self.updates.offer.clone() {
            match update::judge(current(), offer.version, &self.updates.memory, now) {
                Verdict::Offer(_) => return self.open_update_window(&offer, now),
                // Installed meanwhile (by hand, or by the window): nothing to offer any more.
                Verdict::UpToDate => {
                    self.updates.offer = None;
                    adapters::clear_offer();
                }
                Verdict::Skipped | Verdict::Waiting { .. } => {}
            }
        }
        if self.updates.memory.check_due(now) {
            self.start_check(false);
        }
    }

    fn update_busy(&self) -> bool {
        let others = self.stage.shown().notice.is_some_and(|n| {
            !matches!(
                n.kind,
                NoticeKind::UpdateChecking | NoticeKind::UpdateDone | NoticeKind::UpdateAnswer
            )
        });
        others
            || self.fullscreen
            || self.recording_on()
            || self.recording.countdown.is_some()
            || self.selecting()
            || self.has_card()
            || self.expanded
            || self.stage.shown().menu
            || self.scheduler.has_alert()
            || self.shots.viewing.is_some()
            || self.watch.watching.is_some()
    }

    pub(super) fn on_update_command(&mut self, code: usize) {
        let Some(command) = Command::from_code(code) else {
            return;
        };
        let now = persist::now_ms();
        match command {
            Command::Check if !self.updates.working() => self.start_check(true),
            Command::Check => self.make_check_manual(),
            Command::OpenWindow => {
                if let Some(offer) = self.updates.offer.clone() {
                    self.open_update_window(&offer, now);
                }
            }
            Command::Install => self.start_install(),
            Command::Postpone => self.postpone_update(now),
            Command::Skip => self.skip_update(),
        }
    }

    fn postpone_update(&mut self, now: u64) {
        let Some(offer) = self.updates.offer.clone() else {
            return;
        };
        if self.updates.memory.postpone(offer.version, now).is_some() {
            self.save_update_memory();
            self.say_postponed();
        }
    }

    fn skip_update(&mut self) {
        let Some(offer) = self.updates.offer.clone() else {
            return;
        };
        if !update::Bump::between(current(), offer.version).is_some_and(update::Bump::can_skip) {
            return;
        }
        self.updates.memory.skip(offer.version);
        self.save_update_memory();
        self.updates.offer = None;
        adapters::clear_offer();
        self.updates.status = Status::default();
        self.publish_update();
        self.say(
            UpdateLook::Skipped,
            tr!("update.skipped", version = offer.version).into_owned(),
            tr!("update.skipped_hint").into_owned(),
        );
    }

    fn start_check(&mut self, manual: bool) {
        self.updates.checked = true;
        let (sender, receiver) = mpsc::channel();
        // A HWND can't cross threads, a number can.
        let target = self.island.0 as isize;
        let spawned = std::thread::Builder::new()
            .name("update check".into())
            .spawn(move || {
                let _ = sender.send(adapters::fetch_manifest());
                post_to(target);
            });
        if let Err(e) = spawned {
            return self.finish_check(Err(e.into()), manual);
        }
        self.updates.check = Some(receiver);
        self.updates.spinning_since = self.now_ms();
        self.updates.status = Status {
            phase: Phase::Checking,
            manual,
            ..Status::default()
        };
        self.publish_update();
        self.say(
            UpdateLook::Checking,
            tr!("update.checking").into_owned(),
            String::new(),
        );
    }

    /// The user asked while the clock's check was under way: its answer becomes theirs, so a release found
    /// opens the window at once and a skipped or postponed one is offered again.
    fn make_check_manual(&mut self) {
        if self.updates.check.is_none() && self.updates.answered.is_none() {
            return;
        }
        if let Some((_, manual)) = self.updates.answered.as_mut() {
            *manual = true;
        }
        self.updates.status.manual = true;
        self.publish_update();
    }

    /// A thread has something to say.
    pub(super) fn on_update_message(&mut self) {
        self.take_check();
        self.drain_install();
    }

    fn take_check(&mut self) {
        let Some(answer) = self.updates.check.as_ref().and_then(|c| c.try_recv().ok()) else {
            return;
        };
        self.updates.check = None;
        let manual = self.updates.status.manual;
        self.updates.answered = Some((answer, manual));
        self.settle_answer();
    }

    /// The answer is shown once the spinner has turned; if it came sooner, a timer brings it back.
    fn settle_answer(&mut self) {
        if self.updates.answered.is_none() {
            return;
        }
        let spun = self.now_ms().saturating_sub(self.updates.spinning_since);
        if spun < MIN_SPIN_MS {
            self.set_timer(super::timers::UPDATE, (MIN_SPIN_MS - spun) as u32 + 10);
            return;
        }
        if let Some((answer, manual)) = self.updates.answered.take() {
            self.finish_check(answer, manual);
        }
    }

    pub(super) fn on_update_timer(&mut self) {
        self.kill_timer(super::timers::UPDATE);
        self.settle_answer();
    }

    fn finish_check(&mut self, answer: Answer, manual: bool) {
        let now = persist::now_ms();
        let mut changed = self.stage.end(NoticeKind::UpdateChecking, self.now_ms());
        match answer {
            Ok(manifest) => {
                self.updates.said = false;
                self.updates.memory.checked(now);
                self.save_update_memory();
                let verdict = if manual {
                    update::judge_manual(current(), manifest.version, &self.updates.memory, now)
                } else {
                    update::judge(current(), manifest.version, &self.updates.memory, now)
                };
                match verdict {
                    Verdict::UpToDate => {
                        self.updates.offer = None;
                        adapters::clear_offer();
                        self.updates.status = Status {
                            phase: Phase::UpToDate,
                            manual,
                            ..Status::default()
                        };
                        changed |= self.say_done();
                    }
                    // The spinner was up: it ends with an answer, never in silence.
                    Verdict::Skipped => {
                        self.updates.status = Status::default();
                        self.say(
                            UpdateLook::Skipped,
                            tr!("update.skipped", version = manifest.version).into_owned(),
                            tr!("update.skipped_hint").into_owned(),
                        );
                    }
                    Verdict::Offer(_) | Verdict::Waiting { .. } => {
                        journal::info(tr!("journal.update_found", version = manifest.version));
                        if let Err(failed) = adapters::save_offer(&manifest) {
                            journal::warn(tr!(
                                "journal.update_check_failed",
                                error = failed.detail
                            ));
                        }
                        self.updates.offer = Some(manifest.clone());
                        self.set_available(&manifest, manual);
                        match verdict {
                            // Something that started meanwhile (a recording, a selection) is not
                            // covered: then the window waits for the next calm moment.
                            Verdict::Offer(_) if manual || !self.update_busy() => {
                                self.open_update_window(&manifest, now)
                            }
                            Verdict::Waiting { .. } => self.say_postponed(),
                            _ => {}
                        }
                    }
                }
            }
            Err(failed) => {
                self.updates.retry_at = now + RETRY_MS;
                // The network may not be up yet when the island starts with Windows: the check at start is tried again.
                self.updates.checked = manual;
                self.updates.status = Status {
                    phase: Phase::Failed,
                    manual,
                    error: Some(failed.detail.clone()),
                    ..Status::default()
                };
                if manual {
                    // The user asked and did not get it: the island says so, like for any other button.
                    self.report(Fault::new(What::UpdateCheck, failed));
                } else if !self.updates.said {
                    self.updates.said = true;
                    journal::warn(tr!("journal.update_check_failed", error = failed.detail));
                }
            }
        }
        self.publish_update();
        self.after_stage(changed);
    }

    fn offer_view(&self, manifest: &Manifest) -> Option<OfferView> {
        let now = persist::now_ms();
        match update::judge_manual(current(), manifest.version, &self.updates.memory, now) {
            Verdict::Offer(offer) => Some(OfferView {
                current: current(),
                latest: manifest.version,
                notes: manifest.notes.clone(),
                can_skip: offer.can_skip,
                can_postpone: offer.can_postpone,
                skipped: self.updates.memory.skipped == Some(manifest.version),
                size: manifest.installer.size,
            }),
            _ => None,
        }
    }

    fn set_available(&mut self, manifest: &Manifest, manual: bool) {
        self.updates.status = Status {
            phase: Phase::Available,
            manual,
            offer: self.offer_view(manifest),
            ..Status::default()
        };
    }

    fn open_update_window(&mut self, offer: &Manifest, now: u64) {
        if self.has_dialog(Owner::Update) || self.updates.install.is_some() || self.updates.demo {
            return;
        }
        self.updates.memory.shown(offer.version, now);
        self.save_update_memory();
        let manual = self.updates.status.manual;
        self.set_available(offer, manual);
        self.publish_update();
        self.show_update_dialog();
    }

    fn show_update_dialog(&mut self) {
        if let Some(sheet) = sheet::sheet(&self.updates.status)
            && let Err(failed) = self.show_dialog(Owner::Update, sheet)
        {
            self.update_dialog_failed(failed);
        }
    }

    /// After every change of the status: an open window shows it, or goes when there is nothing to show.
    fn sync_update_dialog(&mut self) {
        if !self.has_dialog(Owner::Update) {
            return;
        }
        match sheet::sheet(&self.updates.status) {
            Some(_) => self.show_update_dialog(),
            None => self.close_dialog(Owner::Update),
        }
    }

    pub(super) fn update_dialog_failed(&mut self, failed: Failed) {
        self.updates.retry_at = persist::now_ms() + RETRY_MS;
        journal::warn(tr!("journal.update_window_failed", error = failed.detail));
    }

    pub(super) fn on_update_answer(&mut self, action: Action) {
        let Some(reply) = Reply::of(action) else {
            return;
        };
        let now = persist::now_ms();
        match reply {
            Reply::Install if self.updates.demo => self.start_demo_download(),
            Reply::Install => self.start_install(),
            _ if self.updates.demo => self.end_demo(),
            Reply::Postpone => {
                self.close_dialog(Owner::Update);
                self.postpone_update(now);
            }
            Reply::Skip => {
                self.close_dialog(Owner::Update);
                self.skip_update();
            }
            Reply::Close => self.close_dialog(Owner::Update),
        }
    }

    /// Closed by Alt+F4, with no answer: the offer stays, and the window comes back later.
    pub(super) fn on_update_dialog_closed(&mut self) {
        if self.updates.demo {
            return self.end_demo();
        }
        if self.updates.status.phase == Phase::Available && self.updates.install.is_none() {
            self.updates.retry_at = persist::now_ms() + RETRY_MS;
        }
    }

    fn start_install(&mut self) {
        let Some(offer) = self.updates.offer.clone() else {
            return;
        };
        if self.updates.working() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let target = self.island.0 as isize;
        let total = offer.installer.size;
        let spawned = std::thread::Builder::new()
            .name("update download".into())
            .spawn(move || {
                let report = |step: Step| {
                    let _ = sender.send(step);
                    post_to(target);
                };
                match download(&offer, &report) {
                    Ok(path) => report(Step::Ready(path)),
                    Err(failed) => report(Step::Failed(failed)),
                }
            });
        if let Err(e) = spawned {
            return self.install_failed(e.into());
        }
        self.updates.install = Some(receiver);
        self.updates.status = Status {
            phase: Phase::Downloading,
            offer: self.updates.status.offer.clone(),
            total,
            ..Status::default()
        };
        self.publish_update();
    }

    fn drain_install(&mut self) {
        loop {
            let Some(step) = self
                .updates
                .install
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok())
            else {
                return;
            };
            match step {
                Step::Bytes { done, total } => {
                    self.updates.status.phase = Phase::Downloading;
                    self.updates.status.done = done;
                    if total > 0 {
                        self.updates.status.total = total;
                    }
                    let now = self.now_ms();
                    if now.saturating_sub(self.updates.written) >= WRITE_EVERY_MS {
                        self.updates.written = now;
                        self.publish_update();
                    }
                }
                Step::Failed(failed) => return self.install_failed(failed),
                Step::Ready(path) => return self.run_installer(path),
            }
        }
    }

    fn run_installer(&mut self, path: PathBuf) {
        self.updates.install = None;
        self.updates.status.phase = Phase::Installing;
        self.publish_update();
        let version = self
            .updates
            .offer
            .as_ref()
            .map(|o| o.version.to_string())
            .unwrap_or_default();
        journal::info(tr!("journal.update_installing", version = version));
        // The installer closes the island by itself if it is still there, but this way nothing is lost.
        self.save();
        match process::run_installer(&path) {
            Ok(()) => self.quit(),
            Err(failed) => self.install_failed(failed),
        }
    }

    fn install_failed(&mut self, failed: Failed) {
        self.updates.install = None;
        self.updates.status.phase = Phase::InstallFailed;
        self.updates.status.error = Some(failed.detail.clone());
        self.publish_update();
        if !self.updates.demo {
            self.report(Fault::new(What::UpdateInstall, failed));
        }
    }

    fn say_done(&mut self) -> bool {
        self.updates.notice = Some((
            UpdateLook::UpToDate,
            tr!("update.up_to_date").into_owned(),
            String::new(),
        ));
        self.stage.post(NoticeKind::UpdateDone, self.now_ms())
    }

    /// Shows a notice about the update; `Checking` is held until the check ends.
    fn say(&mut self, look: UpdateLook, title: String, reason: String) {
        self.updates.notice = Some((look, title, reason));
        let kind = match look {
            UpdateLook::Checking => NoticeKind::UpdateChecking,
            UpdateLook::UpToDate => NoticeKind::UpdateDone,
            UpdateLook::Postponed | UpdateLook::Skipped => NoticeKind::UpdateAnswer,
        };
        let changed = self.stage.post(kind, self.now_ms());
        self.after_stage(changed);
    }

    fn say_postponed(&mut self) {
        let memory = &self.updates.memory;
        let (Some(until), Some(version)) = (memory.postponed_until, memory.offered) else {
            return;
        };
        let reason = match memory.deadline(version) {
            Some(deadline) if until < deadline => tr!(
                "update.postponed_until",
                when = platform::local_time(deadline).date_time()
            )
            .into_owned(),
            _ => tr!("update.postponed_last").into_owned(),
        };
        let title = tr!(
            "update.postponed",
            time = platform::local_time(until).clock()
        )
        .into_owned();
        self.say(UpdateLook::Postponed, title, reason);
    }

    fn publish_update(&mut self) {
        self.updates.status.at = persist::now_ms();
        // The demo's sample release is not for the settings to see.
        if !self.updates.demo
            && let Err(failed) = adapters::save_status(&self.updates.status)
        {
            journal::warn(tr!("journal.update_check_failed", error = failed.detail));
        }
        self.sync_update_dialog();
    }

    /// `--demo-update`: the window with the newest release of the changelog, past every check.
    pub(super) fn demo_update(&mut self) {
        if self.updates.working() || self.has_dialog(Owner::Update) {
            return;
        }
        let next = Version {
            patch: current().patch + 1,
            ..current()
        };
        self.updates.demo = true;
        self.updates.status = Status {
            phase: Phase::Available,
            manual: true,
            offer: Some(OfferView {
                current: current(),
                latest: next,
                notes: Notes {
                    en: newest_notes(include_str!("../../../CHANGELOG.en.md")),
                    ru: newest_notes(include_str!("../../../CHANGELOG.md")),
                    uk: newest_notes(include_str!("../../../CHANGELOG.uk.md")),
                },
                can_skip: true,
                can_postpone: true,
                skipped: false,
                size: DEMO_SIZE,
            }),
            ..Status::default()
        };
        self.publish_update();
        self.show_update_dialog();
    }

    /// The demo's download: the bar fills in a few seconds, then the failure page.
    fn start_demo_download(&mut self) {
        if self.updates.working() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let target = self.island.0 as isize;
        let spawned = std::thread::Builder::new()
            .name("update demo".into())
            .spawn(move || {
                const STEPS: u64 = 20;
                for step in 1..=STEPS {
                    std::thread::sleep(Duration::from_millis(150));
                    let _ = sender.send(Step::Bytes {
                        done: DEMO_SIZE * step / STEPS,
                        total: DEMO_SIZE,
                    });
                    post_to(target);
                }
                let _ = sender.send(Step::Failed(Failed::because(
                    Cause::Unknown,
                    "a demo download, nothing to install",
                )));
                post_to(target);
            });
        if spawned.is_err() {
            return;
        }
        self.updates.install = Some(receiver);
        self.updates.status = Status {
            phase: Phase::Downloading,
            offer: self.updates.status.offer.clone(),
            total: DEMO_SIZE,
            ..Status::default()
        };
        self.publish_update();
    }

    fn end_demo(&mut self) {
        self.updates.install = None;
        self.updates.status = Status::default();
        self.publish_update();
        self.close_dialog(Owner::Update);
        self.updates.demo = false;
    }

    fn save_update_memory(&mut self) {
        if let Err(failed) = adapters::save_memory(&self.updates.memory) {
            journal::warn(tr!("journal.update_check_failed", error = failed.detail));
        }
    }
}

/// Wakes the island: a thread has something to say.
fn post_to(target: isize) {
    unsafe {
        let _ = PostMessageW(
            Some(HWND(target as *mut _)),
            WM_UPDATE,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// On the thread: the installer to a file, then its size and signature.
fn download(offer: &Manifest, report: &dyn Fn(Step)) -> Result<PathBuf, Failed> {
    let path =
        adapters::download_dir().join(format!("Great-Island_{}_x64-setup.exe", offer.version));
    let _ = std::fs::remove_file(&path);
    let wanted = offer.installer.size;
    let mut sent = 0;
    http::download(&offer.installer.url, &path, |done, total| {
        // Not on every piece: the island needs a few updates a second.
        if done - sent >= 128 * 1024 {
            sent = done;
            report(Step::Bytes {
                done,
                total: if total > 0 { total } else { wanted },
            });
        }
    })?;
    let bytes = std::fs::read(&path)?;
    let checked = signature::verify(&bytes, &offer.installer.signature).and_then(|()| {
        if wanted > 0 && bytes.len() as u64 != wanted {
            Err(Failed::because(
                crate::fault::Cause::Unknown,
                "the installer is not the one that was announced",
            ))
        } else {
            Ok(())
        }
    });
    if let Err(failed) = checked {
        // A file that is not what was signed must not stay where it could be run.
        let _ = std::fs::remove_file(&path);
        return Err(failed);
    }
    Ok(path)
}
