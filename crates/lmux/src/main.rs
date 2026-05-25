mod app;
mod bus;
mod compositor_bridge;
mod keymap;
mod layout;
mod pane;
mod render;
#[cfg(target_os = "linux")]
mod satellite;
#[cfg(not(target_os = "linux"))]
mod satellite {
    use gtk4::prelude::*;
    use gtk4::Frame;

    use crate::layout::PaneId;
    use crate::pane::{FocusCallback, FocusModeCell};

    pub struct SatelliteWidget {
        pane_id: PaneId,
        frame: Frame,
    }

    impl SatelliteWidget {
        #[allow(dead_code)]
        pub fn placeholder(pane_id: PaneId) -> Self {
            let frame = Frame::builder().hexpand(true).vexpand(true).build();
            frame.add_css_class("pane");
            frame.add_css_class("satellite");
            Self { pane_id, frame }
        }

        pub fn pane_id(&self) -> PaneId {
            self.pane_id
        }

        pub fn widget(&self) -> &Frame {
            &self.frame
        }

        pub fn grab_focus(&self) {
            self.frame.grab_focus();
        }

        pub fn has_exited(&self) -> bool {
            false
        }

        pub fn request_close(&self) {}

        pub fn attach_focus_callback(&self, _cb: FocusCallback, _focus_mode: FocusModeCell) {}
    }
}
mod sidebar;
mod state;
mod switcher;
mod tracing_setup;

use anyhow::Result;
use gtk4::prelude::*;
use gtk4::{gio, glib, Application};

const APP_ID: &str = "no.jpro.lmux";

fn main() -> Result<()> {
    glib::set_prgname(Some("lmux"));
    glib::set_application_name("lmux");

    // Hidden subcommand: `lmux --exec-pty <cmd> <args...>`. Used internally
    // as a trampoline by `lmux-pty::spawn` so every PTY child has
    // `PR_SET_PDEATHSIG(SIGTERM)` set before it execs the real shell
    // (Story 7.3 / FR34 / NFR8). Not user-facing.
    let mut args_iter = std::env::args_os().skip(1);
    if let Some(first) = args_iter.next() {
        if first.as_os_str() == std::ffi::OsStr::new("--exec-pty") {
            pty_trampoline(args_iter.collect());
        }
        if first.as_os_str() == std::ffi::OsStr::new("--request-permissions") {
            request_permissions();
            return Ok(());
        }
    }

    let _log_guard = tracing_setup::init();
    tracing_setup::install_panic_hook();
    tracing_setup::log_startup_banner();

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::empty())
        .build();

    app.connect_activate(app::activate);

    let exit_code = app.run();
    let code = exit_code.value();
    if code != 0 {
        anyhow::bail!("gtk::Application exited with status {code}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn request_permissions() {
    let state = lmux_compositor::MacWindowCompositor::accessibility_permission_state(true);
    println!("macOS Accessibility permission: {state:?}");
}

#[cfg(not(target_os = "macos"))]
fn request_permissions() {
    println!("No platform permissions are required for this build.");
}

fn pty_trampoline(rest: Vec<std::ffi::OsString>) -> ! {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: prctl(2) with PR_SET_PDEATHSIG is Linux-specific. Other Unix
    // platforms still use this trampoline as a plain exec wrapper.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong);
    }

    if rest.is_empty() {
        eprintln!("lmux --exec-pty: missing command");
        std::process::exit(2);
    }

    let mut c_args: Vec<CString> = Vec::with_capacity(rest.len());
    for arg in &rest {
        match CString::new(arg.as_bytes()) {
            Ok(c) => c_args.push(c),
            Err(err) => {
                eprintln!("lmux --exec-pty: invalid argument: {err}");
                std::process::exit(2);
            }
        }
    }
    let mut c_argv: Vec<*const libc::c_char> = c_args.iter().map(|c| c.as_ptr()).collect();
    c_argv.push(std::ptr::null());

    // SAFETY: c_argv lives until execvp returns (if it does, it's an error).
    unsafe {
        libc::execvp(c_argv[0], c_argv.as_ptr());
    }
    let err = std::io::Error::last_os_error();
    eprintln!("lmux --exec-pty: execvp failed: {err}");
    std::process::exit(127);
}
