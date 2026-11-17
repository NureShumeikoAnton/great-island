# Changelog

English mirror of [CHANGELOG.md](CHANGELOG.md), which is the original and is written in Russian. Format: loosely [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org/).
New entries go at the top under "Unreleased" and move under a version number when an installer is built. Versions before 0.6.1 are only in the Russian file.

## Unreleased

### Changed
- The project is renamed from Break Island to Great Island. The new name is everywhere: windows, the tray icon, the installer (`Great Island_…_x64-setup.exe`), `great-island.exe`, the settings folder `%APPDATA%\com.greatisland.app`, the registry key `Software\Great Island`. Settings of the old version are not carried over; uninstall the old "Break Island" before installing.

### Added
- The license is GPL-3.0-or-later (the `LICENSE` file): the code may be used and changed, but a changed version stays open.

## 0.6.1 — 2026-11-17

Fixes after the first round of testing, and a choice of the monitor for the island.

### Added
- The "Island on" setting: by default the island follows the cursor to the monitor you are working on, or it can be pinned to one of the monitors. The island moves only while it rests quietly (not expanded, no card, no recording, no viewer window, no area selection); it checks once a second. A screenshot or recording started with a key takes the monitor under the cursor at the moment of the press. A reminder appears on the pinned monitor, or, when nothing is pinned, where the cursor is. If the pinned monitor is unplugged, the island stays on the main one.
- A journal for diagnosing other people's computers: "a media player appeared: …" for every new Windows player, and a "loudness for …" line when the island did not find the player's sound (Yandex Browser); when the Sound or Microphone button of the control center is refused, the journal and the notice name the device and the Windows error code.

### Fixed
- The expanded island with no reminders no longer clips the "Turn on reminders in Settings" text: the island is wider than the text.
- Video window: after the window closes, the background reading of thumbnails and sound for the editor stops instead of reading the recording to the end (each reader held a decoder of its own on the video card).
- Video editor: fragments with a transition no longer lie over each other. The blocks sit side by side, as without a transition, and the transition is a circle between them; each block gives half of its length to the transition.
- "Show in folder" for a file that no longer exists (deleted or moved) opens the folder it was in. If the folder is gone too, the island reports an error.
- Video editor: a large fragment can be dropped into the trash by any part: it is enough that it touches the trash, not only its middle.
