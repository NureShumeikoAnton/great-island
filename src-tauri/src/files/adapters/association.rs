//! The island as a program for photos and videos: in Explorer's "Open with" for every type
//! the viewer reads, and on Windows' Default apps page. Windows doesn't let a program make
//! itself the default since Windows 8; the user picks it there. All in HKCU: no admin rights.

use crate::{
    fault::{Cause, Failed},
    files::open::{PHOTOS, VIDEOS},
    lang::tr,
};
use windows::{
    Win32::{
        Foundation::{ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW,
            RegGetValueW, RegSetKeyValueW,
        },
        UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify},
    },
    core::{HSTRING, PCWSTR},
};

const APP: &str = "Great Island";
const CLASSES: &str = r"Software\Classes";
const CAPABILITIES: &str = r"Software\Great Island\Capabilities";
const REGISTERED: &str = r"Software\RegisteredApplications";
const KINDS: [(&str, &[&str]); 2] = [
    ("GreatIsland.Photo", &PHOTOS),
    ("GreatIsland.Video", &VIDEOS),
];
/// The page of this app among Windows' Default apps (Windows 11; Windows 10 opens the list).
pub const DEFAULT_APPS: &str = "ms-settings:defaultapps?registeredAppUser=Great%20Island";

fn command(exe: &str) -> String {
    format!("\"{exe}\" --open \"%1\"")
}

/// Writes what is missing or points elsewhere (the exe moved); true = something changed. Run
/// at every start, so a moved or reinstalled island keeps opening files.
pub fn register() -> Result<bool, Failed> {
    let exe = std::env::current_exe()?;
    let exe_name = exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let exe = exe.display().to_string();
    let command = command(&exe);
    let description = tr!("association.description");
    let current = KINDS.iter().all(|(id, _)| {
        read(&format!(r"{CLASSES}\{id}\shell\open\command"), "").as_deref() == Some(&command)
    }) && read(REGISTERED, APP).as_deref() == Some(CAPABILITIES)
        && read(CAPABILITIES, "ApplicationDescription").as_deref() == Some(&description);
    if current {
        return Ok(false);
    }
    let app = format!(r"{CLASSES}\Applications\{exe_name}");
    write(&format!(r"{app}\shell\open\command"), "", &command)?;
    write(&app, "FriendlyAppName", APP)?;
    for (id, types) in KINDS {
        let class = format!(r"{CLASSES}\{id}");
        write(&format!(r"{class}\shell\open\command"), "", &command)?;
        write(
            &format!(r"{class}\DefaultIcon"),
            "",
            &format!("\"{exe}\",0"),
        )?;
        write(&format!(r"{class}\Application"), "ApplicationName", APP)?;
        for ext in types {
            write(&format!(r"{CLASSES}\.{ext}\OpenWithProgids"), id, "")?;
            write(&format!(r"{app}\SupportedTypes"), &format!(".{ext}"), "")?;
            write(
                &format!(r"{CAPABILITIES}\FileAssociations"),
                &format!(".{ext}"),
                id,
            )?;
        }
    }
    write(CAPABILITIES, "ApplicationName", APP)?;
    write(CAPABILITIES, "ApplicationDescription", &description)?;
    write(REGISTERED, APP, CAPABILITIES)?;
    told();
    Ok(true)
}

/// For the uninstaller: everything `register` wrote. What Windows itself keeps about the
/// user's choice (UserChoice) falls back to Windows' default once the program is gone.
pub fn unregister() {
    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    unsafe {
        for (id, types) in KINDS {
            let _ = RegDeleteTreeW(
                HKEY_CURRENT_USER,
                &HSTRING::from(format!(r"{CLASSES}\{id}")),
            );
            for ext in types {
                let _ = RegDeleteKeyValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(format!(r"{CLASSES}\.{ext}\OpenWithProgids")),
                    &HSTRING::from(id),
                );
            }
        }
        let app = format!(r"{CLASSES}\Applications\{exe_name}");
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(app));
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(CAPABILITIES));
        let _ = RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(REGISTERED),
            &HSTRING::from(APP),
        );
    }
    told();
}

/// Explorer rereads its menus and icons.
fn told() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

fn read(key: &str, name: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let mut size = 0u32;
    unsafe {
        if RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut text = vec![0u16; (size as usize).div_ceil(2)];
        let read = RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut size),
        );
        (read == ERROR_SUCCESS).then(|| {
            String::from_utf16_lossy(&text)
                .trim_end_matches('\0')
                .to_string()
        })
    }
}

fn write(key: &str, name: &str, value: &str) -> Result<(), Failed> {
    let text: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let name = HSTRING::from(name);
    let done = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            if name.is_empty() {
                PCWSTR::null()
            } else {
                PCWSTR(name.as_ptr())
            },
            REG_SZ.0,
            Some(text.as_ptr().cast()),
            (text.len() * 2) as u32,
        )
    };
    match done {
        ERROR_SUCCESS => Ok(()),
        WIN32_ERROR(code) => Err(Failed::because(
            Cause::from_os(code),
            format_args!("{key}: {code}"),
        )),
    }
}
