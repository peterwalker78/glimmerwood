//! What the companion is given to work with here: the window on screen,
//! Windows' idea of where files live, and a timer.

use std::path::PathBuf;
use std::rc::Rc;

use glimmerwood_core::companion;
use glimmerwood_core::dose::Moment;
use glimmerwood_core::host;
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SW_SHOWNORMAL, SetTimer};
use windows::core::{GUID, HSTRING, PCWSTR, PWSTR, w};

use crate::shell::Shell;

/// The companion's own timer on the main window.
pub const WAKE: usize = 2;

impl host::Host for Shell {
    /// Hand a file to whatever Windows opens that kind of file with. Only
    /// ever a download that has finished, and never a page.
    fn open_file(&self, path: &str) -> bool {
        let file = HSTRING::from(path);
        let opened = unsafe {
            ShellExecuteW(
                Some(self.window()),
                w!("open"),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // It answers with an error code below 33, and something of no
        // meaning above it, rather than saying plainly whether it worked.
        opened.0 as usize > 32
    }

    fn windows(&self) -> Vec<Rc<dyn host::Window>> {
        match crate::shell::held() {
            Some(shell) => vec![shell as Rc<dyn host::Window>],
            None => Vec::new(),
        }
    }

    /// The one place this build reads the wall clock, which is what a host
    /// is for; everything above it is handed the moment it concerns.
    #[allow(clippy::disallowed_methods)]
    fn now(&self) -> Moment {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as i64);
        Moment {
            ms,
            utc_offset_s: utc_offset_s(),
        }
    }

    fn data_dir(&self) -> PathBuf {
        known_folder(&FOLDERID_LocalAppData).unwrap_or_else(|| PathBuf::from("."))
    }

    fn config_dir(&self) -> PathBuf {
        config_dir()
    }

    fn home_dir(&self) -> PathBuf {
        known_folder(&FOLDERID_Profile).unwrap_or_else(|| PathBuf::from("."))
    }

    fn locale_country(&self) -> Option<String> {
        let mut name = [0u16; MAX_PATH as usize];
        let written = unsafe { GetUserDefaultLocaleName(&mut name) };
        if written <= 0 {
            return None;
        }
        // Windows writes `en-GB`; the parser knows `en_GB`.
        let locale = String::from_utf16_lossy(&name[..(written - 1) as usize]).replace('-', "_");
        companion::country_from_locales(&[locale])
    }

    fn wake_in(&self, ms: u64) {
        unsafe {
            let _ = KillTimer(Some(self.window()), WAKE);
            SetTimer(
                Some(self.window()),
                WAKE,
                ms.clamp(1, u32::MAX as u64) as u32,
                None,
            );
        }
    }

    #[allow(clippy::disallowed_methods)]
    fn noise(&self) -> u32 {
        // Enough unpredictability to start a garden's shape from.
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos() as u64);
        (ms ^ (ms >> 32)) as u32
    }
}

/// How far ahead of UTC the machine's clock is, in seconds.
fn utc_offset_s() -> i32 {
    let mut zone = TIME_ZONE_INFORMATION::default();
    let which = unsafe { GetTimeZoneInformation(&mut zone) };
    // Windows gives the bias as the minutes to *add* to local time to reach
    // UTC, which is the other way round from everyone else.
    // GetTimeZoneInformation answers 2 when daylight saving is in force.
    const DAYLIGHT: u32 = 2;
    let bias = zone.Bias
        + if which == DAYLIGHT {
            zone.DaylightBias
        } else {
            zone.StandardBias
        };
    -bias * 60
}

/// Where this user's settings live. Also wanted before there is a shell to
/// ask, for the size the window opens at.
pub fn config_dir() -> PathBuf {
    known_folder(&FOLDERID_RoamingAppData).unwrap_or_else(|| PathBuf::from("."))
}

fn known_folder(which: &GUID) -> Option<PathBuf> {
    let path: PWSTR = unsafe { SHGetKnownFolderPath(which, KF_FLAG_DEFAULT, None) }.ok()?;
    let text = unsafe { path.to_string() }.ok()?;
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(path.as_ptr().cast())) };
    Some(PathBuf::from(text))
}
