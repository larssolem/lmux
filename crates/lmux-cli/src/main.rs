use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "lmux-cli", version, about = "lmux control CLI")]
struct Cli {
    /// Emit machine-readable JSON for commands that support it.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Promote the pane running this command to the session's anchor.
    /// Equivalent to pressing `Ctrl+B` then `a` in the pane.
    MarkAnchor,
    /// tmux-compatible alias for `pane capture`.
    CapturePane {
        #[arg(short = 't', long = "target")]
        target: String,
        #[arg(short = 'n', long, default_value_t = 80)]
        lines: u32,
    },
    /// tmux-compatible alias for `pane send`.
    SendKeys {
        #[arg(short = 't', long = "target")]
        target: String,
        #[arg(required = true, trailing_var_arg = true)]
        keys: Vec<String>,
    },
    /// tmux-compatible alias for `pane new --tab`.
    NewWindow {
        #[arg(short = 't', long = "target", default_value = "current")]
        target: String,
        #[arg(short = 'n', long = "name")]
        name: Option<String>,
        #[arg(trailing_var_arg = true)]
        argv: Vec<String>,
    },
    /// Session management.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Anchor control. Target anchors by UUID, shown in the sidebar popover.
    #[command(subcommand)]
    Anchor(AnchorCommand),
    /// Live pane inventory. Lists every pane's UUID so the user can feed
    /// it into `anchor tag` without having to copy from the sidebar.
    #[command(subcommand)]
    Pane(PaneCommand),
    /// GUI satellite control.
    #[command(subcommand)]
    Satellite(SatelliteCommand),
    /// MCP adapter discovery and client configuration.
    #[command(subcommand)]
    Mcp(McpCommand),
    /// lmux process snapshot: pid, version, anchor count, session count,
    /// compositor state. Routed through the bus (`status.get`).
    Status,
}

#[derive(Subcommand, Debug)]
enum AnchorCommand {
    /// List anchors/workspaces.
    List,
    /// Print the active anchor UUID.
    Active,
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
    /// Create a terminal pane in the current or selected anchor.
    New {
        /// Anchor UUID, or `current` for the active anchor.
        #[arg(long, default_value = "current")]
        anchor: String,
        /// Place the new pane as a tab once tab stacks are available.
        #[arg(long, conflicts_with_all = ["split_right", "split_down"])]
        tab: bool,
        /// Split the target anchor to the right.
        #[arg(long = "split-right", conflicts_with_all = ["tab", "split_down"])]
        split_right: bool,
        /// Split the target anchor downward.
        #[arg(long = "split-down", conflicts_with_all = ["tab", "split_right"])]
        split_down: bool,
        /// Visible pane title.
        #[arg(long)]
        name: Option<String>,
        /// Agent-visible purpose string.
        #[arg(long)]
        purpose: Option<String>,
        /// Focus the new pane after creation.
        #[arg(long)]
        activate: bool,
        /// Optional command argv to send to the new shell.
        #[arg(trailing_var_arg = true)]
        argv: Vec<String>,
    },
    /// Print recent transcript output for a terminal pane UUID.
    Tail {
        uuid: String,
        #[arg(long, default_value_t = 80)]
        lines: u32,
    },
    /// Print transcript output newer than a sequence number.
    Capture {
        uuid: String,
        #[arg(long)]
        since: Option<u64>,
        #[arg(long)]
        max_lines: Option<u32>,
    },
    /// Send text to a terminal pane UUID.
    Send {
        uuid: String,
        text: String,
        /// Append Enter after the provided text.
        #[arg(long)]
        enter: bool,
    },
    /// Rename a pane and pin the title as user-provided.
    Rename { uuid: String, title: String },
}

#[derive(Subcommand, Debug)]
enum SatelliteCommand {
    /// Legacy spawn path. Prefer `list-windows` + `attach-window`.
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
    /// List native windows that can be attached.
    ListWindows,
    /// Attach a specific native window to the active anchor.
    AttachWindow {
        /// Window backend from `satellite list-windows`, such as `kwin`, `x11`, or `macos`.
        #[arg(long)]
        backend: String,
        /// Backend window id from `satellite list-windows`.
        #[arg(long)]
        backend_window_id: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long)]
        window_id: Option<i64>,
        #[arg(long)]
        window_index: Option<u32>,
        #[arg(long)]
        bundle_id: Option<String>,
        #[arg(long)]
        app_identity_kind: Option<String>,
        #[arg(long)]
        app_identity_value: Option<String>,
        #[arg(long)]
        title: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum McpCommand {
    /// Show whether lmux-mcp and known AI CLI clients are available.
    Status,
    /// Configure a known AI CLI client to launch lmux-mcp.
    Install {
        /// Client to configure. `auto` installs every supported client found in PATH.
        #[arg(long, value_enum, default_value_t = McpClientSelection::Auto)]
        client: McpClientSelection,
        /// Print commands without executing them.
        #[arg(long)]
        dry_run: bool,
    },
    /// Print MCP server config snippets for manual setup.
    PrintConfig {
        /// Output format.
        #[arg(long, value_enum, default_value_t = McpConfigFormat::Json)]
        format: McpConfigFormat,
        /// Agent id advertised by lmux-mcp in grant prompts.
        #[arg(long, default_value = "lmux-mcp")]
        agent_id: String,
        /// Agent name advertised by lmux-mcp in grant prompts.
        #[arg(long, default_value = "lmux MCP")]
        agent_name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum McpClientSelection {
    Auto,
    Codex,
    Claude,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum McpConfigFormat {
    Json,
    ClaudeProject,
    CodexToml,
}

#[derive(Debug)]
struct AttachWindowOptions {
    backend: String,
    backend_window_id: Option<String>,
    pid: Option<u32>,
    window_id: Option<i64>,
    window_index: Option<u32>,
    bundle_id: Option<String>,
    app_identity_kind: Option<String>,
    app_identity_value: Option<String>,
    title: Option<String>,
}

#[derive(Debug)]
struct PaneNewOptions {
    anchor: String,
    tab: bool,
    split_right: bool,
    split_down: bool,
    name: Option<String>,
    purpose: Option<String>,
    activate: bool,
    argv: Vec<String>,
    json: bool,
}

#[derive(Subcommand, Debug)]
enum SessionCommand {
    /// List sessions, most-recently-opened first.
    List,
    /// Create a new empty session. Routed through the bus so lmux
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
    /// Swap lmux's live pane tree for the named session's
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
    let json = cli.json;
    match cli.command {
        Command::MarkAnchor => run_mark_anchor(),
        Command::CapturePane { target, lines } => run_pane_tail(&target, lines, json),
        Command::SendKeys { target, keys } => run_send_keys_alias(&target, keys),
        Command::NewWindow { target, name, argv } => run_pane_new(PaneNewOptions {
            anchor: target,
            tab: true,
            split_right: false,
            split_down: false,
            name,
            purpose: None,
            activate: true,
            argv,
            json,
        }),
        Command::Session(SessionCommand::List) => run_session_list(),
        Command::Session(SessionCommand::New { name }) => run_session_new(&name),
        Command::Session(SessionCommand::Rename { from, to }) => run_session_rename(&from, &to),
        Command::Session(SessionCommand::Delete { name }) => run_session_delete(&name),
        Command::Session(SessionCommand::Open { name }) => run_session_open(&name),
        Command::Anchor(AnchorCommand::List) => run_anchor_list(json),
        Command::Anchor(AnchorCommand::Active) => run_anchor_active(json),
        Command::Anchor(AnchorCommand::New) => run_anchor_new(),
        Command::Anchor(AnchorCommand::Activate { uuid }) => run_anchor_activate(&uuid),
        Command::Anchor(AnchorCommand::Pause { uuid }) => run_anchor_pause(&uuid),
        Command::Anchor(AnchorCommand::Resume { uuid }) => run_anchor_resume(&uuid),
        Command::Anchor(AnchorCommand::Hide { uuid }) => run_anchor_hide(&uuid),
        Command::Anchor(AnchorCommand::Reattach { uuid }) => run_anchor_reattach(&uuid),
        Command::Anchor(AnchorCommand::Untag { uuid }) => run_anchor_untag(&uuid),
        Command::Anchor(AnchorCommand::Tag { uuid }) => run_anchor_tag(&uuid),
        Command::Pane(PaneCommand::List) => run_pane_list(),
        Command::Pane(PaneCommand::New {
            anchor,
            tab,
            split_right,
            split_down,
            name,
            purpose,
            activate,
            argv,
        }) => run_pane_new(PaneNewOptions {
            anchor,
            tab,
            split_right,
            split_down,
            name,
            purpose,
            activate,
            argv,
            json,
        }),
        Command::Pane(PaneCommand::Tail { uuid, lines }) => run_pane_tail(&uuid, lines, json),
        Command::Pane(PaneCommand::Capture {
            uuid,
            since,
            max_lines,
        }) => run_pane_capture(&uuid, since, max_lines, json),
        Command::Pane(PaneCommand::Send { uuid, text, enter }) => {
            run_pane_send(&uuid, &text, enter)
        }
        Command::Pane(PaneCommand::Rename { uuid, title }) => run_pane_rename(&uuid, &title),
        Command::Satellite(SatelliteCommand::Open { target, argv }) => {
            run_satellite_open(&target, argv)
        }
        Command::Satellite(SatelliteCommand::AttachFocused) => run_satellite_attach_focused(),
        Command::Satellite(SatelliteCommand::ListWindows) => run_satellite_list_windows(),
        Command::Satellite(SatelliteCommand::AttachWindow {
            backend,
            backend_window_id,
            pid,
            window_id,
            window_index,
            bundle_id,
            app_identity_kind,
            app_identity_value,
            title,
        }) => run_satellite_attach_window(AttachWindowOptions {
            backend,
            backend_window_id,
            pid,
            window_id,
            window_index,
            bundle_id,
            app_identity_kind,
            app_identity_value,
            title,
        }),
        Command::Mcp(McpCommand::Status) => run_mcp_status(json),
        Command::Mcp(McpCommand::Install { client, dry_run }) => run_mcp_install(client, dry_run),
        Command::Mcp(McpCommand::PrintConfig {
            format,
            agent_id,
            agent_name,
        }) => run_mcp_print_config(format, &agent_id, &agent_name),
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
                let pid = window
                    .pid
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".into());
                let app = format_app_identity(window.app_identity.as_ref());
                let title = window.title.unwrap_or_default();
                println!(
                    "backend={:?} backend_window_id={} pid={} app={} title={}",
                    window.backend, window.backend_window_id, pid, app, title
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

fn run_satellite_attach_window(options: AttachWindowOptions) -> ExitCode {
    let AttachWindowOptions {
        backend,
        backend_window_id,
        pid,
        window_id,
        window_index,
        bundle_id,
        app_identity_kind,
        app_identity_value,
        title,
    } = options;
    let backend = match parse_window_backend(&backend) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            return ExitCode::from(2);
        }
    };
    let backend_window_id = match backend_window_id
        .or_else(|| legacy_macos_backend_window_id(pid, window_id, window_index))
    {
        Some(value) => value,
        None => {
            eprintln!("lmux-cli: --backend-window-id is required");
            return ExitCode::from(2);
        }
    };
    let app_identity = match parse_app_identity(bundle_id, app_identity_kind, app_identity_value) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            return ExitCode::from(2);
        }
    };
    match run_bus_write(lmux_bus::Kind::SatelliteAttachWindow {
        backend,
        backend_window_id,
        pid,
        app_identity,
        title,
        workspace: None,
        output: None,
        agent: agent_from_env(),
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

fn parse_window_backend(backend: &str) -> Result<lmux_bus::kinds::WindowCandidateBackend, String> {
    match backend {
        "macos" => Ok(lmux_bus::kinds::WindowCandidateBackend::Macos),
        "kwin" => Ok(lmux_bus::kinds::WindowCandidateBackend::Kwin),
        "x11" => Ok(lmux_bus::kinds::WindowCandidateBackend::X11),
        "hyprland" => Ok(lmux_bus::kinds::WindowCandidateBackend::Hyprland),
        "sway" => Ok(lmux_bus::kinds::WindowCandidateBackend::Sway),
        "noop" => Ok(lmux_bus::kinds::WindowCandidateBackend::Noop),
        "unsupported" => Ok(lmux_bus::kinds::WindowCandidateBackend::Unsupported),
        other => Err(format!("unknown window backend: {other}")),
    }
}

fn legacy_macos_backend_window_id(
    pid: Option<u32>,
    window_id: Option<i64>,
    window_index: Option<u32>,
) -> Option<String> {
    match (pid, window_id, window_index) {
        (_, Some(window_id), Some(index)) => {
            Some(format!("macos-window-id:{window_id}:index:{index}"))
        }
        (Some(pid), None, Some(index)) => Some(format!("macos-window-pid:{pid}:index:{index}")),
        _ => None,
    }
}

fn parse_app_identity(
    bundle_id: Option<String>,
    kind: Option<String>,
    value: Option<String>,
) -> Result<Option<lmux_bus::kinds::WindowAppIdentity>, String> {
    if let Some(bundle_id) = bundle_id {
        return Ok(Some(lmux_bus::kinds::WindowAppIdentity::BundleId(
            bundle_id,
        )));
    }
    let Some(kind) = kind else {
        return Ok(None);
    };
    let Some(value) = value else {
        return Err("--app-identity-value is required with --app-identity-kind".into());
    };
    let identity = match kind.as_str() {
        "bundle_id" | "bundle-id" => lmux_bus::kinds::WindowAppIdentity::BundleId(value),
        "desktop_entry" | "desktop-entry" => {
            lmux_bus::kinds::WindowAppIdentity::DesktopEntry(value)
        }
        "wm_class" | "wm-class" => lmux_bus::kinds::WindowAppIdentity::WmClass(value),
        "app_id" | "app-id" => lmux_bus::kinds::WindowAppIdentity::AppId(value),
        "other" => lmux_bus::kinds::WindowAppIdentity::Other(value),
        other => return Err(format!("unknown app identity kind: {other}")),
    };
    Ok(Some(identity))
}

fn format_app_identity(identity: Option<&lmux_bus::kinds::WindowAppIdentity>) -> String {
    match identity {
        Some(lmux_bus::kinds::WindowAppIdentity::BundleId(value)) => format!("bundle_id:{value}"),
        Some(lmux_bus::kinds::WindowAppIdentity::DesktopEntry(value)) => {
            format!("desktop_entry:{value}")
        }
        Some(lmux_bus::kinds::WindowAppIdentity::WmClass(value)) => format!("wm_class:{value}"),
        Some(lmux_bus::kinds::WindowAppIdentity::AppId(value)) => format!("app_id:{value}"),
        Some(lmux_bus::kinds::WindowAppIdentity::Other(value)) => format!("other:{value}"),
        None => "-".into(),
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
        no_sandbox: true,
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

fn run_anchor_list(json: bool) -> ExitCode {
    match run_bus_request(lmux_bus::Kind::AnchorList {}) {
        Ok(lmux_bus::Kind::AnchorListResult { anchors }) => {
            if json {
                return print_json(&anchors);
            }
            if anchors.is_empty() {
                println!("(no anchors)");
                return ExitCode::SUCCESS;
            }
            for anchor in anchors {
                let active = if anchor.active { "*" } else { " " };
                let pane = anchor
                    .pane_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".into());
                println!(
                    "{active} {}  pane={}  {}",
                    anchor.anchor_id, pane, anchor.label
                );
            }
            ExitCode::SUCCESS
        }
        Ok(other) => unexpected_response(other),
        Err(err) => print_error(err),
    }
}

fn run_anchor_active(json: bool) -> ExitCode {
    match run_bus_request(lmux_bus::Kind::AnchorList {}) {
        Ok(lmux_bus::Kind::AnchorListResult { anchors }) => {
            let active = anchors.into_iter().find(|anchor| anchor.active);
            if json {
                return print_json(&active);
            }
            match active {
                Some(anchor) => {
                    println!("{}", anchor.anchor_id);
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("lmux-cli: no active anchor");
                    ExitCode::from(1)
                }
            }
        }
        Ok(other) => unexpected_response(other),
        Err(err) => print_error(err),
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

fn run_pane_new(options: PaneNewOptions) -> ExitCode {
    let target_anchor = if options.anchor == "current" {
        None
    } else {
        match parse_uuid(&options.anchor) {
            Ok(parsed) => Some(parsed),
            Err(code) => return code,
        }
    };
    let placement = if options.tab {
        lmux_bus::PanePlacement::Tab
    } else if options.split_down {
        lmux_bus::PanePlacement::SplitDown
    } else {
        let _ = options.split_right;
        lmux_bus::PanePlacement::SplitRight
    };
    match run_bus_request(lmux_bus::Kind::PaneNew {
        target_anchor,
        placement,
        activate: options.activate,
        title: options.name,
        argv: options.argv,
        agent: agent_from_env(),
        purpose: options.purpose,
    }) {
        Ok(lmux_bus::Kind::PaneNewResult(created)) => {
            if options.json {
                return print_json(&created);
            }
            println!(
                "pane: {}  anchor: {}  placement: {:?}",
                created.pane_id, created.anchor_id, created.placement
            );
            ExitCode::SUCCESS
        }
        Ok(other) => unexpected_response(other),
        Err(err) => print_error(err),
    }
}

fn run_pane_tail(uuid: &str, lines: u32, json: bool) -> ExitCode {
    let pane_id = match parse_uuid(uuid) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    match run_bus_request(lmux_bus::Kind::PaneTail {
        pane_id,
        lines,
        agent: agent_from_env(),
    }) {
        Ok(lmux_bus::Kind::PaneTranscriptResult(range)) => print_transcript_range(range, json),
        Ok(other) => unexpected_response(other),
        Err(err) => print_error(err),
    }
}

fn run_pane_capture(
    uuid: &str,
    since_sequence: Option<u64>,
    max_lines: Option<u32>,
    json: bool,
) -> ExitCode {
    let pane_id = match parse_uuid(uuid) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    match run_bus_request(lmux_bus::Kind::PaneCapture {
        pane_id,
        since_sequence,
        max_lines,
        agent: agent_from_env(),
    }) {
        Ok(lmux_bus::Kind::PaneTranscriptResult(range)) => print_transcript_range(range, json),
        Ok(other) => unexpected_response(other),
        Err(err) => print_error(err),
    }
}

fn print_transcript_range(range: lmux_bus::TranscriptRange, json: bool) -> ExitCode {
    if json {
        return print_json(&range);
    }
    if range.truncated {
        eprintln!("lmux-cli: transcript truncated before requested sequence");
    }
    for line in range.lines {
        println!("{}", line.text);
    }
    ExitCode::SUCCESS
}

fn run_pane_send(uuid: &str, text: &str, enter: bool) -> ExitCode {
    let pane_id = match parse_uuid(uuid) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    let mut text = text.to_string();
    if enter {
        text.push('\n');
    }
    match run_bus_write(lmux_bus::Kind::PaneSendInput {
        pane_id,
        text,
        agent: agent_from_env(),
    }) {
        Ok(()) => {
            println!("sent: {uuid}");
            ExitCode::SUCCESS
        }
        Err(err) => print_error(err),
    }
}

fn run_send_keys_alias(uuid: &str, keys: Vec<String>) -> ExitCode {
    let mut text = String::new();
    for key in keys {
        match key.as_str() {
            "Enter" | "enter" | "Return" | "return" => text.push('\n'),
            other => {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push(' ');
                }
                text.push_str(other);
            }
        }
    }
    run_pane_send(uuid, &text, false)
}

fn run_pane_rename(uuid: &str, title: &str) -> ExitCode {
    let pane_id = match parse_uuid(uuid) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    match run_bus_write(lmux_bus::Kind::PaneRename {
        pane_id,
        title: title.to_string(),
        pin: true,
        agent: None,
    }) {
        Ok(()) => {
            println!("renamed: {uuid}");
            ExitCode::SUCCESS
        }
        Err(err) => print_error(err),
    }
}

#[derive(Debug, serde::Serialize)]
struct McpStatus {
    lmux_mcp: ToolAvailability,
    codex: ToolAvailability,
    claude: ToolAvailability,
}

#[derive(Debug, serde::Serialize)]
struct ToolAvailability {
    command: String,
    path: Option<String>,
    available: bool,
}

fn run_mcp_status(json: bool) -> ExitCode {
    let status = McpStatus {
        lmux_mcp: lmux_mcp_availability(),
        codex: tool_availability("codex"),
        claude: tool_availability("claude"),
    };
    if json {
        return print_json(&status);
    }
    print_tool_status("lmux-mcp", &status.lmux_mcp);
    print_tool_status("codex", &status.codex);
    print_tool_status("claude", &status.claude);
    ExitCode::SUCCESS
}

fn print_tool_status(label: &str, tool: &ToolAvailability) {
    let state = if tool.available { "found" } else { "missing" };
    let path = tool.path.as_deref().unwrap_or("-");
    println!("{label}: {state} ({path})");
}

fn run_mcp_install(selection: McpClientSelection, dry_run: bool) -> ExitCode {
    let lmux_mcp = lmux_mcp_availability();
    let lmux_mcp_command = lmux_mcp.path.as_deref().unwrap_or("lmux-mcp").to_string();
    if dry_run {
        for client in selected_mcp_clients(selection) {
            println!(
                "{}",
                shell_command_line(&client.install_command(&lmux_mcp_command))
            );
        }
        return ExitCode::SUCCESS;
    }

    if !lmux_mcp.available {
        eprintln!(
            "lmux-cli: lmux-mcp not found in PATH or next to lmux-cli; build/install lmux-mcp first"
        );
        return ExitCode::from(1);
    }

    let clients = selected_mcp_clients(selection);
    let mut attempted = 0usize;
    let mut failed = false;
    for client in clients {
        let command = client.install_command(&lmux_mcp_command);
        if !tool_availability(&command[0]).available {
            if selection != McpClientSelection::Auto {
                eprintln!("lmux-cli: {} not found in PATH", command[0]);
                return ExitCode::from(1);
            }
            continue;
        }
        attempted = attempted.saturating_add(1);
        match std::process::Command::new(&command[0])
            .args(&command[1..])
            .status()
        {
            Ok(status) if status.success() => {
                println!("installed lmux MCP for {}", client.name());
            }
            Ok(status) => {
                eprintln!("lmux-cli: {} exited with {status}", client.name());
                failed = true;
            }
            Err(err) => {
                eprintln!("lmux-cli: failed to run {}: {err}", command[0]);
                failed = true;
            }
        }
    }

    if attempted == 0 {
        eprintln!("lmux-cli: no supported MCP clients found in PATH");
        return ExitCode::from(1);
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn run_mcp_print_config(format: McpConfigFormat, agent_id: &str, agent_name: &str) -> ExitCode {
    let command = lmux_mcp_availability()
        .path
        .unwrap_or_else(|| "lmux-mcp".to_string());
    match format {
        McpConfigFormat::Json | McpConfigFormat::ClaudeProject => {
            let value = serde_json::json!({
                "mcpServers": {
                    "lmux": {
                        "command": command,
                        "args": [],
                        "env": {
                            "LMUX_AGENT_ID": agent_id,
                            "LMUX_AGENT_NAME": agent_name,
                        }
                    }
                }
            });
            print_json(&value)
        }
        McpConfigFormat::CodexToml => {
            println!("[mcp_servers.lmux]");
            println!("command = \"{}\"", toml_escape(&command));
            println!("args = []");
            println!("[mcp_servers.lmux.env]");
            println!("LMUX_AGENT_ID = \"{}\"", toml_escape(agent_id));
            println!("LMUX_AGENT_NAME = \"{}\"", toml_escape(agent_name));
            ExitCode::SUCCESS
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum SupportedMcpClient {
    Codex,
    Claude,
}

impl SupportedMcpClient {
    fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn install_command(self, lmux_mcp_command: &str) -> Vec<String> {
        let mut command: Vec<String> = match self {
            Self::Codex => vec![
                "codex",
                "mcp",
                "add",
                "--env",
                "LMUX_AGENT_ID=codex",
                "--env",
                "LMUX_AGENT_NAME=Codex",
                "lmux",
                "--",
            ],
            Self::Claude => vec![
                "claude",
                "mcp",
                "add",
                "--scope",
                "user",
                "-e",
                "LMUX_AGENT_ID=claude",
                "-e",
                "LMUX_AGENT_NAME=Claude",
                "lmux",
                "--",
            ],
        }
        .into_iter()
        .map(str::to_string)
        .collect();
        command.push(lmux_mcp_command.to_string());
        command
    }
}

fn selected_mcp_clients(selection: McpClientSelection) -> Vec<SupportedMcpClient> {
    match selection {
        McpClientSelection::Auto => vec![SupportedMcpClient::Codex, SupportedMcpClient::Claude],
        McpClientSelection::Codex => vec![SupportedMcpClient::Codex],
        McpClientSelection::Claude => vec![SupportedMcpClient::Claude],
    }
}

fn tool_availability(command: &str) -> ToolAvailability {
    let path = find_in_path(command).map(|path| path.display().to_string());
    ToolAvailability {
        command: command.to_string(),
        available: path.is_some(),
        path,
    }
}

fn lmux_mcp_availability() -> ToolAvailability {
    let path = find_in_path("lmux-mcp")
        .or_else(lmux_mcp_next_to_current_exe)
        .map(|path| path.display().to_string());
    ToolAvailability {
        command: "lmux-mcp".to_string(),
        available: path.is_some(),
        path,
    }
}

fn lmux_mcp_next_to_current_exe() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("lmux-mcp");
    is_executable_file(&candidate).then_some(candidate)
}

fn find_in_path(command: &str) -> Option<std::path::PathBuf> {
    if command.contains(std::path::MAIN_SEPARATOR) {
        let path = std::path::PathBuf::from(command);
        return is_executable_file(&path).then_some(path);
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(command))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn shell_command_line(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':' | b'=')
        })
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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

fn parse_uuid(value: &str) -> Result<uuid::Uuid, ExitCode> {
    value.parse::<uuid::Uuid>().map_err(|err| {
        eprintln!("lmux-cli: invalid UUID: {err}");
        ExitCode::from(2)
    })
}

fn agent_from_env() -> Option<lmux_bus::AgentIdentity> {
    let id = std::env::var("LMUX_AGENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let name = std::env::var("LMUX_AGENT_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    Some(lmux_bus::AgentIdentity { id, name })
}

fn print_json<T: serde::Serialize>(value: &T) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("lmux-cli: json: {err}");
            ExitCode::from(1)
        }
    }
}

fn print_error(err: anyhow::Error) -> ExitCode {
    eprintln!("lmux-cli: {err}");
    ExitCode::from(1)
}

fn unexpected_response(other: lmux_bus::Kind) -> ExitCode {
    eprintln!("lmux-cli: unexpected bus response: {other:?}");
    ExitCode::from(1)
}

fn run_bus_request(kind: lmux_bus::Kind) -> anyhow::Result<lmux_bus::Kind> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let mut client = lmux_bus::Client::connect_default(lmux_bus::ClientRole::LmuxCli).await?;
        client.request(kind).await.map_err(Into::into)
    })
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
    match run_bus_request(lmux_bus::Kind::AnchorTagSelf {}) {
        Ok(lmux_bus::Kind::AnchorTagSelfResult { pane_id }) => {
            println!("anchor: {pane_id}");
            ExitCode::SUCCESS
        }
        Ok(other) => unexpected_response(other),
        Err(err) => {
            eprintln!("lmux-cli: {err}");
            ExitCode::from(1)
        }
    }
}
