use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use lmux_control::{
    send_request, socket_path, Error as CtrlError, Request, Response, PROTOCOL_VERSION,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Parser, Debug)]
#[command(name = "lmux-cli", version, about = "lmux control CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Promote the pane running this command to the session's anchor.
    /// Equivalent to pressing `Super+A` on the pane (FR19 / FR36).
    MarkAnchor,
    /// Session management (FR1-FR6, FR63).
    #[command(subcommand)]
    Session(SessionCommand),
    /// Anchor control (FR19 / FR36). Target anchor by UUID — shown under
    /// each entry in the sidebar popover.
    #[command(subcommand)]
    Anchor(AnchorCommand),
    /// Live pane inventory. Lists every pane's UUID so the user can feed
    /// it into `anchor tag` without having to copy from the sidebar.
    #[command(subcommand)]
    Pane(PaneCommand),
    /// GUI satellite control (Epic 9). v0.2 ships `open` only — `detach`
    /// and geometry follow with v0.3.
    #[command(subcommand)]
    Satellite(SatelliteCommand),
    /// Cockpit snapshot: pid, version, anchor count, session count,
    /// compositor state. Routed through the bus (`status.get`).
    Status,
}

#[derive(Subcommand, Debug)]
enum AnchorCommand {
    /// Create a new anchor pane and make it active.
    New,
    /// Activate an existing anchor by its anchor UUID.
    Activate { uuid: String },
    /// Pause the backing process (SIGSTOP to the process group).
    Pause { uuid: String },
    /// Resume a previously paused anchor (SIGCONT).
    Resume { uuid: String },
    /// Hide the widget of a tagged anchor; PTY + scrollback stay alive.
    Hide { uuid: String },
    /// Restore a hidden anchor's widget (inverse of `hide`).
    Reattach { uuid: String },
    /// Untag an anchor so its pane reverts to a regular terminal.
    Untag { uuid: String },
    /// Tag a pane as an anchor by its pane UUID.
    Tag { uuid: String },
}

#[derive(Subcommand, Debug)]
enum PaneCommand {
    /// List every live pane, its UUID, whether it's tagged as an anchor,
    /// and its last-known cwd.
    List,
}

#[derive(Subcommand, Debug)]
enum SatelliteCommand {
    /// Spawn a GUI satellite. The first positional argument is the
    /// executable, remaining args are forwarded. The cockpit stamps
    /// `LMUX_SATELLITE_ID=<uuid>` on the child's environment so the
    /// compositor script can correlate the new window.
    Open {
        /// Target pane UUID the satellite should dock to once the KWin
        /// script-side correlation lands (v0.3). v0.2 accepts it but
        /// doesn't yet wire geometry.
        #[arg(long, default_value = "00000000-0000-0000-0000-000000000000")]
        target: String,
        /// Executable + args.
        #[arg(required = true, trailing_var_arg = true)]
        argv: Vec<String>,
    },
    /// Attach the currently focused native macOS window to the active anchor.
    AttachFocused,
    /// List native macOS windows that can be attached.
    ListWindows,
    /// Attach a specific native macOS window to the active anchor.
    AttachWindow {
        #[arg(long)]
        pid: u32,
        #[arg(long)]
        window_id: Option<i64>,
        #[arg(long)]
        window_index: Option<u32>,
        #[arg(long)]
        bundle_id: Option<String>,
        #[arg(long)]
        title: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCommand {
    /// List sessions, most-recently-opened first.
    List,
    /// Create a new empty session. Routed through the bus so the cockpit
    /// sees the new entry without a rescan.
    New {
        /// Session name — letters, digits, `-`, `_` only.
        name: String,
    },
    /// Rename a session. Both names follow the same slug rules as `new`.
    Rename { from: String, to: String },
    /// Delete a session. The snapshot on disk is removed along with the
    /// index entry.
    Delete { name: String },
    /// Swap the cockpit's live pane tree for the named session's
    /// snapshot. Equivalent to picking it in the Ctrl+B s switcher.
    Open { name: String },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::MarkAnchor => run_mark_anchor(),
        Command::Session(SessionCommand::List) => run_session_list(),
        Command::Session(SessionCommand::New { name }) => run_session_new(&name),
        Command::Session(SessionCommand::Rename { from, to }) => run_session_rename(&from, &to),
        Command::Session(SessionCommand::Delete { name }) => run_session_delete(&name),
        Command::Session(SessionCommand::Open { name }) => run_session_open(&name),
        Command::Anchor(AnchorCommand::New) => run_anchor_new(),
        Command::Anchor(AnchorCommand::Activate { uuid }) => run_anchor_activate(&uuid),
        Command::Anchor(AnchorCommand::Pause { uuid }) => run_anchor_pause(&uuid),
        Command::Anchor(AnchorCommand::Resume { uuid }) => run_anchor_resume(&uuid),
        Command::Anchor(AnchorCommand::Hide { uuid }) => run_anchor_hide(&uuid),
        Command::Anchor(AnchorCommand::Reattach { uuid }) => run_anchor_reattach(&uuid),
        Command::Anchor(AnchorCommand::Untag { uuid }) => run_anchor_untag(&uuid),
        Command::Anchor(AnchorCommand::Tag { uuid }) => run_anchor_tag(&uuid),
        Command::Pane(PaneCommand::List) => run_pane_list(),
        Command::Satellite(SatelliteCommand::Open { target, argv }) => {
            run_satellite_open(&target, argv)
        }
        Command::Satellite(SatelliteCommand::AttachFocused) => run_satellite_attach_focused(),
        Command::Satellite(SatelliteCommand::ListWindows) => run_satellite_list_windows(),
        Command::Satellite(SatelliteCommand::AttachWindow {
            pid,
            window_id,
            window_index,
            bundle_id,
            title,
        }) => run_satellite_attach_window(pid, window_id, window_index, bundle_id, title),
        Command::Status => run_status(),
    }
}

fn run_satellite_attach_focused() -> ExitCode {
    match run_bus_write(lmux_bus::Kind::SatelliteAttachFocused {}) {
        Ok(()) => {
            println!("focused window attached");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_satellite_list_windows() -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            return ExitCode::from(1);
        }
    };
    let res = rt.block_on(async {
        let mut client = lmux_bus::Client::connect_default(lmux_bus::ClientRole::LmuxCli).await?;
        client
            .request(lmux_bus::Kind::SatelliteListWindows {})
            .await
    });
    match res {
        Ok(lmux_bus::Kind::SatelliteListWindowsResult { windows }) => {
            for window in windows {
                let window_id = window
                    .window_id
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".into());
                let bundle = window.bundle_id.unwrap_or_else(|| "-".into());
                let title = window.title.unwrap_or_default();
                println!(
                    "pid={} window_id={} index={} bundle={} title={}",
                    window.pid, window_id, window.window_index, bundle, title
                );
            }
            ExitCode::SUCCESS
        }
        Ok(other) => {
            eprintln!("lmux-cli: unexpected bus response: {other:?}");
            ExitCode::from(1)
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_satellite_attach_window(
    pid: u32,
    window_id: Option<i64>,
    window_index: Option<u32>,
    bundle_id: Option<String>,
    title: Option<String>,
) -> ExitCode {
    match run_bus_write(lmux_bus::Kind::SatelliteAttachWindow {
        pid,
        window_id,
        window_index,
        bundle_id,
        title,
    }) {
        Ok(()) => {
            println!("window attached");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_satellite_open(target: &str, argv: Vec<String>) -> ExitCode {
    let target_uuid = match target.parse::<uuid::Uuid>() {
        Ok(u) => u,
        Err(err) => {
            eprintln!("lmux-cli: invalid target UUID: {err}");
            return ExitCode::from(2);
        }
    };
    match run_bus_write(lmux_bus::Kind::SatelliteOpen {
        argv,
        target_pane: target_uuid,
        no_sandbox: false,
    }) {
        Ok(()) => {
            println!("satellite spawned");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_anchor_new() -> ExitCode {
    match run_bus_write(lmux_bus::Kind::AnchorNew {}) {
        Ok(()) => {
            println!("anchor created");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_anchor_activate(uuid: &str) -> ExitCode {
    match uuid.parse::<uuid::Uuid>() {
        Ok(parsed) => match run_bus_write(lmux_bus::Kind::AnchorActivate { pane_id: parsed }) {
            Ok(()) => {
                println!("activated: {uuid}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("lmux-cli: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("lmux-cli: invalid UUID: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_status() -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            return ExitCode::from(1);
        }
    };
    let res: anyhow::Result<lmux_bus::StatusSnapshot> = rt.block_on(async {
        let mut client = lmux_bus::Client::connect_default(lmux_bus::ClientRole::LmuxCli).await?;
        let resp = client.request(lmux_bus::Kind::StatusGet {}).await?;
        match resp {
            lmux_bus::Kind::StatusGetResult(s) => Ok(s),
            other => Err(anyhow::anyhow!("unexpected bus response: {other:?}")),
        }
    });
    match res {
        Ok(s) => {
            println!("cockpit_version: {}", s.cockpit_version);
            println!("pid: {}", s.pid);
            println!("sessions: {}", s.session_count);
            println!("anchors: {}", s.anchor_count);
            println!("compositor: {:?}", s.compositor);
            println!(
                "satellites: ok={} fail={}",
                s.satellite_spawn_ok, s.satellite_spawn_fail
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_pane_list() -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            return ExitCode::from(1);
        }
    };
    let res: anyhow::Result<Vec<lmux_bus::PaneSummary>> = rt.block_on(async {
        let mut client = lmux_bus::Client::connect_default(lmux_bus::ClientRole::LmuxCli).await?;
        let resp = client.request(lmux_bus::Kind::PaneList {}).await?;
        match resp {
            lmux_bus::Kind::PaneListResult { panes } => Ok(panes),
            other => Err(anyhow::anyhow!("unexpected bus response: {other:?}")),
        }
    });
    match res {
        Ok(panes) => {
            if panes.is_empty() {
                println!("(no panes)");
                return ExitCode::SUCCESS;
            }
            for p in panes {
                let role = match p.anchor_id {
                    Some(a) => format!("anchor {a}"),
                    None => "pane".into(),
                };
                let cwd = p.cwd.as_deref().unwrap_or("-");
                println!("{}  {role}  {cwd}", p.pane_id);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_anchor_pause(uuid: &str) -> ExitCode {
    match uuid.parse::<uuid::Uuid>() {
        Ok(parsed) => match run_bus_write(lmux_bus::Kind::AnchorPause { pane_id: parsed }) {
            Ok(()) => {
                println!("paused: {uuid}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("lmux-cli: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("lmux-cli: invalid UUID: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_anchor_resume(uuid: &str) -> ExitCode {
    match uuid.parse::<uuid::Uuid>() {
        Ok(parsed) => match run_bus_write(lmux_bus::Kind::AnchorResume { pane_id: parsed }) {
            Ok(()) => {
                println!("resumed: {uuid}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("lmux-cli: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("lmux-cli: invalid UUID: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_anchor_hide(uuid: &str) -> ExitCode {
    match uuid.parse::<uuid::Uuid>() {
        Ok(parsed) => match run_bus_write(lmux_bus::Kind::AnchorHide { pane_id: parsed }) {
            Ok(()) => {
                println!("hidden: {uuid}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("lmux-cli: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("lmux-cli: invalid UUID: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_anchor_reattach(uuid: &str) -> ExitCode {
    match uuid.parse::<uuid::Uuid>() {
        Ok(parsed) => match run_bus_write(lmux_bus::Kind::AnchorReattach { pane_id: parsed }) {
            Ok(()) => {
                println!("reattached: {uuid}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("lmux-cli: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("lmux-cli: invalid UUID: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_session_rename(from: &str, to: &str) -> ExitCode {
    match run_bus_write(lmux_bus::Kind::SessionRename {
        from: from.to_string(),
        to: to.to_string(),
    }) {
        Ok(()) => {
            println!("renamed: {from} -> {to}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_session_open(name: &str) -> ExitCode {
    match run_bus_write(lmux_bus::Kind::SessionOpen {
        name: name.to_string(),
    }) {
        Ok(()) => {
            println!("opened: {name}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_anchor_untag(uuid: &str) -> ExitCode {
    match uuid.parse::<uuid::Uuid>() {
        Ok(parsed) => match run_bus_write(lmux_bus::Kind::AnchorUntag { pane_id: parsed }) {
            Ok(()) => {
                println!("untagged: {uuid}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("lmux-cli: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("lmux-cli: invalid UUID: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_anchor_tag(uuid: &str) -> ExitCode {
    match uuid.parse::<uuid::Uuid>() {
        Ok(parsed) => match run_bus_write(lmux_bus::Kind::AnchorTag { pane_id: parsed }) {
            Ok(()) => {
                println!("tagged: {uuid}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("lmux-cli: {err}");
                ExitCode::from(1)
            }
        },
        Err(err) => {
            eprintln!("lmux-cli: invalid UUID: {err}");
            ExitCode::from(2)
        }
    }
}

fn run_session_delete(name: &str) -> ExitCode {
    match run_bus_write(lmux_bus::Kind::SessionDelete {
        name: name.to_string(),
    }) {
        Ok(()) => {
            println!("deleted: {name}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_bus_write(kind: lmux_bus::Kind) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let mut client = lmux_bus::Client::connect_default(lmux_bus::ClientRole::LmuxCli).await?;
        let resp = client.request(kind).await?;
        match resp {
            lmux_bus::Kind::Ok { .. } => Ok(()),
            other => Err(anyhow::anyhow!("unexpected bus response: {other:?}")),
        }
    })
}

fn run_session_new(name: &str) -> ExitCode {
    match run_bus_write(lmux_bus::Kind::SessionNew {
        name: name.to_string(),
    }) {
        Ok(()) => {
            println!("session: {name}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_session_list() -> ExitCode {
    // Prefer the bus when the cockpit is running; otherwise fall back to
    // reading the store directly so `lmux session list` works offline.
    match try_session_list_via_bus() {
        Ok(entries) => {
            print_session_list(&entries);
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::debug!(error = %err, "bus unavailable, falling back to store");
            match try_session_list_via_store() {
                Ok(entries) => {
                    print_session_list(&entries);
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("lmux-cli: {err}");
                    ExitCode::from(1)
                }
            }
        }
    }
}

fn print_session_list(entries: &[(String, u64)]) {
    if entries.is_empty() {
        println!("(no sessions)");
        return;
    }
    for (name, ts) in entries {
        println!("{ts}\t{name}");
    }
}

fn try_session_list_via_bus() -> anyhow::Result<Vec<(String, u64)>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let mut client = lmux_bus::Client::connect_default(lmux_bus::ClientRole::LmuxCli).await?;
        let resp = client.request(lmux_bus::Kind::SessionList {}).await?;
        match resp {
            lmux_bus::Kind::SessionListResult { sessions } => Ok(sessions
                .into_iter()
                .map(|s| (s.name, s.created_at_unix_seconds))
                .collect()),
            other => Err(anyhow::anyhow!(
                "unexpected bus response for session.list: {other:?}"
            )),
        }
    })
}

fn try_session_list_via_store() -> anyhow::Result<Vec<(String, u64)>> {
    let Some(state_home) = lmux_session::state_home() else {
        anyhow::bail!("XDG_STATE_HOME / HOME not set; cannot locate sessions dir");
    };
    let store = lmux_session::SessionStore::new(&state_home);
    if !store.root().exists() {
        return Ok(Vec::new());
    }
    let entries = store.list()?;
    Ok(entries
        .into_iter()
        .map(|e| (e.name, e.last_opened_at_unix_seconds))
        .collect())
}

fn run_mark_anchor() -> ExitCode {
    // Pre-flight: if the socket file isn't there at all, lmux isn't running.
    // This is FR38's "not running" branch and deserves its own exit code so
    // shell wrappers can distinguish it from protocol errors.
    let path = match socket_path() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            return ExitCode::from(2);
        }
    };
    if !path.exists() {
        eprintln!(
            "lmux-cli: lmux not running (no control socket at {})",
            path.display()
        );
        return ExitCode::from(2);
    }

    let source_pid = unsafe { libc::getpid() } as u32;
    let req = Request::MarkAnchor {
        v: PROTOCOL_VERSION,
        source_pid,
    };
    match send_request(&req, CONNECT_TIMEOUT) {
        Ok(Response::Ok { pane_id, .. }) => {
            match pane_id {
                Some(id) => println!("anchor: {id}"),
                None => println!("anchor: set"),
            }
            ExitCode::SUCCESS
        }
        Ok(Response::Error { message, .. }) => {
            eprintln!("lmux-cli: {message}");
            ExitCode::from(4)
        }
        Err(CtrlError::Timeout) => {
            eprintln!("lmux-cli: lmux control socket unresponsive (timed out after 2 s)");
            ExitCode::from(3)
        }
        Err(CtrlError::Io(err))
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            eprintln!(
                "lmux-cli: lmux not running (no control socket at {})",
                path.display()
            );
            ExitCode::from(2)
        }
        Err(CtrlError::Io(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {
            eprintln!("lmux-cli: lmux control socket unresponsive (timed out after 2 s)");
            ExitCode::from(3)
        }
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}
