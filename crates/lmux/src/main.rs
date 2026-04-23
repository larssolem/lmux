mod app;
mod bus;
mod compositor_bridge;
mod keymap;
mod launcher;
mod layout;
mod pane;
mod render;
mod satellite;
mod sidebar;
mod state;
mod switcher;
mod tracing_setup;

use anyhow::Result;
use gtk4::prelude::*;
use gtk4::{gio, Application};

const APP_ID: &str = "no.jpro.lmux";

fn main() -> Result<()> {
    // Hidden subcommand: `lmux --exec-pty <cmd> <args...>`. Used internally
    // as a trampoline by `lmux-pty::spawn` so every PTY child has
    // `PR_SET_PDEATHSIG(SIGTERM)` set before it execs the real shell
    // (Story 7.3 / FR34 / NFR8). Not user-facing.
    let mut args_iter = std::env::args_os().skip(1);
    if args_iter.next().as_deref() == Some(std::ffi::OsStr::new("--exec-pty")) {
        pty_trampoline(args_iter.collect());
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

fn pty_trampoline(rest: Vec<std::ffi::OsString>) -> ! {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: prctl(2) with PR_SET_PDEATHSIG is a no-op on non-Linux but
    // always safe to call on Linux. The ulong is the signal number.
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
