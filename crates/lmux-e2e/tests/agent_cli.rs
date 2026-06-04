//! E2E smoke tests for agent-control CLI commands.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::assert::OutputAssertExt;
use lmux_e2e::Env;
use predicates::prelude::*;

const PANE: &str = "00000000-0000-0000-0000-000000000123";
const ANCHOR: &str = "00000000-0000-0000-0000-000000000456";

#[test]
fn pane_help_lists_agent_commands() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["pane", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tail"))
        .stdout(predicate::str::contains("capture"))
        .stdout(predicate::str::contains("send"))
        .stdout(predicate::str::contains("rename"));
}

#[test]
fn json_flag_is_accepted_for_agent_commands() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["--json", "pane", "tail", PANE, "--lines", "2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("lmux-cli:"));
}

#[test]
fn invalid_uuid_is_rejected_before_bus_mapping() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["pane", "tail", "not-a-uuid"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid UUID"));
}

#[test]
fn tmux_aliases_accept_valid_arguments() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["capture-pane", "-t", PANE, "-n", "5"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("lmux-cli:"));

    env.cli("lmux-cli")
        .args(["send-keys", "-t", PANE, "echo", "ok", "Enter"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("lmux-cli:"));

    env.cli("lmux-cli")
        .args(["new-window", "-t", ANCHOR, "-n", "tests", "cargo", "test"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("lmux-cli:"));
}

#[test]
fn agent_environment_is_accepted_for_bus_mapped_commands() {
    let env = Env::new();
    env.cli("lmux-cli")
        .env("LMUX_AGENT_ID", "agent-1")
        .env("LMUX_AGENT_NAME", "Agent One")
        .args(["pane", "send", PANE, "q"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("lmux-cli:"));
}

#[test]
fn mcp_help_lists_discovery_commands() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["mcp", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("install"))
        .stdout(predicate::str::contains("print-config"));
}

#[test]
fn mcp_install_dry_run_prints_client_commands() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args(["mcp", "install", "--client", "codex", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex mcp add"))
        .stdout(predicate::str::contains("lmux-mcp"));

    env.cli("lmux-cli")
        .args(["mcp", "install", "--client", "claude", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude mcp add"))
        .stdout(predicate::str::contains("LMUX_AGENT_ID=claude"));
}

#[test]
fn mcp_print_config_supports_json_and_codex_toml() {
    let env = Env::new();
    env.cli("lmux-cli")
        .args([
            "mcp",
            "print-config",
            "--format",
            "json",
            "--agent-id",
            "agent-1",
            "--agent-name",
            "Agent One",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mcpServers\""))
        .stdout(predicate::str::contains("\"command\""))
        .stdout(predicate::str::contains("lmux-mcp"))
        .stdout(predicate::str::contains("\"LMUX_AGENT_ID\": \"agent-1\""));

    env.cli("lmux-cli")
        .args(["mcp", "print-config", "--format", "codex-toml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[mcp_servers.lmux]"))
        .stdout(predicate::str::contains("command = "))
        .stdout(predicate::str::contains("lmux-mcp"));
}
