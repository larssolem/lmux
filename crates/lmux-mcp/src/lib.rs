use std::io::{BufRead, Write};

use lmux_bus::{AgentIdentity, Client, ClientRole, Kind, PanePlacement};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

pub fn run_stdio<R: BufRead, W: Write>(mut input: R, mut output: W) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new().map_err(|err| err.to_string())?;
    while let Some(request) = read_message(&mut input)? {
        let response = rt.block_on(handle_json_rpc(request));
        write_message(&mut output, &response)?;
    }
    Ok(())
}

async fn handle_json_rpc(request: Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {"name": "lmux-mcp", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"tools": {}}
        })),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => handle_tool_call(request.get("params").cloned().unwrap_or_default()).await,
        "notifications/initialized" => return Value::Null,
        _ => Err(format!("unsupported method: {method}")),
    };
    match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(err) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": err}}),
    }
}

async fn handle_tool_call(params: Value) -> Result<Value, String> {
    #[derive(Deserialize)]
    struct ToolCall {
        name: String,
        #[serde(default)]
        arguments: Value,
    }
    let call: ToolCall = serde_json::from_value(params).map_err(|err| err.to_string())?;
    let kind = kind_for_tool(&call.name, call.arguments, agent_from_env())?;
    let mut client = Client::connect_default(ClientRole::LmuxMcp)
        .await
        .map_err(|err| err.to_string())?;
    match client.request(kind).await {
        Ok(response) => Ok(tool_text(
            serde_json::to_string_pretty(&response).map_err(|err| err.to_string())?,
        )),
        Err(err) => Ok(json!({
            "isError": true,
            "content": [{"type": "text", "text": err.to_string()}]
        })),
    }
}

fn tool_text(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}]})
}

pub fn tool_definitions() -> Value {
    json!([
        tool(
            "anchor_list",
            "List lmux anchors/workspaces.",
            json!({"type": "object", "properties": {}})
        ),
        tool(
            "pane_new",
            "Create a terminal pane in an anchor.",
            json!({
                "type": "object",
                "properties": {
                    "anchor": {"type": "string", "description": "Anchor UUID or current"},
                    "placement": {"type": "string", "enum": ["tab", "split_right", "split_down"]},
                    "activate": {"type": "boolean"},
                    "title": {"type": "string"},
                    "purpose": {"type": "string"},
                    "argv": {"type": "array", "items": {"type": "string"}}
                }
            })
        ),
        tool(
            "pane_tail",
            "Read recent transcript lines.",
            json!({
                "type": "object",
                "required": ["pane_id"],
                "properties": {"pane_id": {"type": "string"}, "lines": {"type": "integer"}}
            })
        ),
        tool(
            "pane_capture",
            "Read transcript lines since a sequence.",
            json!({
                "type": "object",
                "required": ["pane_id"],
                "properties": {"pane_id": {"type": "string"}, "since_sequence": {"type": "integer"}, "max_lines": {"type": "integer"}}
            })
        ),
        tool(
            "pane_send",
            "Send input to a terminal pane.",
            json!({
                "type": "object",
                "required": ["pane_id", "text"],
                "properties": {"pane_id": {"type": "string"}, "text": {"type": "string"}}
            })
        ),
        tool(
            "pane_rename",
            "Rename a terminal pane.",
            json!({
                "type": "object",
                "required": ["pane_id", "title"],
                "properties": {"pane_id": {"type": "string"}, "title": {"type": "string"}}
            })
        ),
        tool(
            "satellite_list_windows",
            "List native GUI windows available for attach.",
            json!({"type": "object", "properties": {}})
        ),
        tool(
            "satellite_attach_window",
            "Request attaching an existing GUI window.",
            json!({
                "type": "object",
                "required": ["backend", "backend_window_id"],
                "properties": {
                    "backend": {"type": "string", "enum": ["macos", "kwin", "x11", "noop", "unsupported"]},
                    "backend_window_id": {"type": "string"},
                    "pid": {"type": "integer"},
                    "title": {"type": "string"}
                }
            })
        ),
        tool(
            "satellite_launch",
            "Launch a GUI app, wait for a matching native window, then request exact-window attach.",
            json!({
                "type": "object",
                "required": ["argv"],
                "properties": {
                    "argv": {"type": "array", "items": {"type": "string"}},
                    "title_hint": {"type": "string"},
                    "app_hint": {"type": "string"},
                    "timeout_ms": {"type": "integer", "minimum": 250, "maximum": 30000}
                }
            })
        )
    ])
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

pub fn kind_for_tool(
    name: &str,
    args: Value,
    agent: Option<AgentIdentity>,
) -> Result<Kind, String> {
    match name {
        "anchor_list" => Ok(Kind::AnchorList {}),
        "pane_new" => {
            #[derive(Deserialize)]
            struct Args {
                #[serde(default)]
                anchor: Option<String>,
                #[serde(default)]
                placement: Option<String>,
                #[serde(default)]
                activate: bool,
                #[serde(default)]
                title: Option<String>,
                #[serde(default)]
                purpose: Option<String>,
                #[serde(default)]
                argv: Vec<String>,
            }
            let args: Args = serde_json::from_value(args).map_err(|err| err.to_string())?;
            Ok(Kind::PaneNew {
                target_anchor: parse_optional_anchor(args.anchor)?,
                placement: parse_placement(args.placement.as_deref())?,
                activate: args.activate,
                title: args.title,
                argv: args.argv,
                agent,
                purpose: args.purpose,
            })
        }
        "pane_tail" => {
            #[derive(Deserialize)]
            struct Args {
                pane_id: Uuid,
                #[serde(default = "default_tail_lines")]
                lines: u32,
            }
            let args: Args = serde_json::from_value(args).map_err(|err| err.to_string())?;
            Ok(Kind::PaneTail {
                pane_id: args.pane_id,
                lines: args.lines,
                agent,
            })
        }
        "pane_capture" => {
            #[derive(Deserialize)]
            struct Args {
                pane_id: Uuid,
                #[serde(default)]
                since_sequence: Option<u64>,
                #[serde(default)]
                max_lines: Option<u32>,
            }
            let args: Args = serde_json::from_value(args).map_err(|err| err.to_string())?;
            Ok(Kind::PaneCapture {
                pane_id: args.pane_id,
                since_sequence: args.since_sequence,
                max_lines: args.max_lines,
                agent,
            })
        }
        "pane_send" => {
            #[derive(Deserialize)]
            struct Args {
                pane_id: Uuid,
                text: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|err| err.to_string())?;
            Ok(Kind::PaneSendInput {
                pane_id: args.pane_id,
                text: args.text,
                agent,
            })
        }
        "pane_rename" => {
            #[derive(Deserialize)]
            struct Args {
                pane_id: Uuid,
                title: String,
            }
            let args: Args = serde_json::from_value(args).map_err(|err| err.to_string())?;
            Ok(Kind::PaneRename {
                pane_id: args.pane_id,
                title: args.title,
                pin: false,
                agent,
            })
        }
        "satellite_list_windows" => Ok(Kind::SatelliteListWindows {}),
        "satellite_attach_window" => {
            #[derive(Deserialize)]
            struct Args {
                backend: lmux_bus::kinds::WindowCandidateBackend,
                backend_window_id: String,
                #[serde(default)]
                pid: Option<u32>,
                #[serde(default)]
                title: Option<String>,
            }
            let args: Args = serde_json::from_value(args).map_err(|err| err.to_string())?;
            Ok(Kind::SatelliteAttachWindow {
                backend: args.backend,
                backend_window_id: args.backend_window_id,
                pid: args.pid,
                app_identity: None,
                title: args.title,
                workspace: None,
                output: None,
                agent,
            })
        }
        "satellite_launch" => {
            #[derive(Deserialize)]
            struct Args {
                argv: Vec<String>,
                #[serde(default)]
                title_hint: Option<String>,
                #[serde(default)]
                app_hint: Option<String>,
                #[serde(default)]
                timeout_ms: Option<u64>,
            }
            let args: Args = serde_json::from_value(args).map_err(|err| err.to_string())?;
            Ok(Kind::SatelliteLaunchAttach {
                argv: args.argv,
                title_hint: args.title_hint,
                app_hint: args.app_hint,
                timeout_ms: args.timeout_ms,
                agent,
            })
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn default_tail_lines() -> u32 {
    80
}

fn parse_optional_anchor(value: Option<String>) -> Result<Option<Uuid>, String> {
    match value.as_deref() {
        None | Some("current") => Ok(None),
        Some(value) => value
            .parse::<Uuid>()
            .map(Some)
            .map_err(|err| format!("invalid anchor UUID: {err}")),
    }
}

fn parse_placement(value: Option<&str>) -> Result<PanePlacement, String> {
    match value.unwrap_or("split_right") {
        "tab" => Ok(PanePlacement::Tab),
        "split_right" => Ok(PanePlacement::SplitRight),
        "split_down" => Ok(PanePlacement::SplitDown),
        other => Err(format!("invalid placement: {other}")),
    }
}

fn agent_from_env() -> Option<AgentIdentity> {
    let id = std::env::var("LMUX_AGENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "lmux-mcp".to_string());
    let name = std::env::var("LMUX_AGENT_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| Some("lmux MCP".to_string()));
    Some(AgentIdentity { id, name })
}

fn read_message<R: BufRead>(input: &mut R) -> Result<Option<Value>, String> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let read = input.read_line(&mut line).map_err(|err| err.to_string())?;
        if read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|err| format!("invalid Content-Length: {err}"))?,
            );
        }
    }
    let len = content_length.ok_or_else(|| "missing Content-Length".to_string())?;
    if len > MAX_MESSAGE_BYTES {
        return Err(format!(
            "Content-Length {len} exceeds maximum {MAX_MESSAGE_BYTES}"
        ));
    }
    let mut body = vec![0_u8; len];
    input.read_exact(&mut body).map_err(|err| err.to_string())?;
    serde_json::from_slice(&body).map_err(|err| err.to_string())
}

fn write_message<W: Write>(output: &mut W, value: &Value) -> Result<(), String> {
    if value.is_null() {
        return Ok(());
    }
    let body = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len()).map_err(|err| err.to_string())?;
    output.write_all(&body).map_err(|err| err.to_string())?;
    output.flush().map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn agent() -> AgentIdentity {
        AgentIdentity {
            id: "agent".into(),
            name: Some("Agent".into()),
        }
    }

    #[test]
    fn tool_list_contains_core_tools() {
        let tools = tool_definitions();
        let names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        assert!(names.contains(&"anchor_list"));
        assert!(names.contains(&"pane_new"));
        assert!(names.contains(&"satellite_attach_window"));
    }

    #[test]
    fn pane_tail_maps_to_bus_kind_with_agent() {
        let pane_id = Uuid::new_v4();
        let kind = kind_for_tool(
            "pane_tail",
            json!({"pane_id": pane_id, "lines": 12}),
            Some(agent()),
        )
        .unwrap();
        match kind {
            Kind::PaneTail {
                pane_id: actual,
                lines,
                agent,
            } => {
                assert_eq!(actual, pane_id);
                assert_eq!(lines, 12);
                assert_eq!(agent.unwrap().id, "agent");
            }
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[test]
    fn satellite_launch_maps_to_launch_attach_with_hints() {
        let kind = kind_for_tool(
            "satellite_launch",
            json!({
                "argv": ["kate", "--new-window"],
                "title_hint": "notes",
                "app_hint": "org.kde.kate",
                "timeout_ms": 1200
            }),
            Some(agent()),
        )
        .unwrap();
        match kind {
            Kind::SatelliteLaunchAttach {
                argv,
                title_hint,
                app_hint,
                timeout_ms,
                agent,
            } => {
                assert_eq!(argv, vec!["kate", "--new-window"]);
                assert_eq!(title_hint.as_deref(), Some("notes"));
                assert_eq!(app_hint.as_deref(), Some("org.kde.kate"));
                assert_eq!(timeout_ms, Some(1200));
                assert_eq!(agent.unwrap().id, "agent");
            }
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[test]
    fn framed_stdio_roundtrip_initialize() {
        let request = br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let mut input = Vec::new();
        input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", request.len()).as_bytes());
        input.extend_from_slice(request);
        let mut output = Vec::new();
        run_stdio(std::io::Cursor::new(input), &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("Content-Length:"));
        assert!(text.contains("lmux-mcp"));
    }
}
