//! Bridge the limux control socket onto the GTK host state.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use gtk4 as gtk;
use limux_control::auth::{self, SocketControlMode};
use limux_control::request_io::{self, read_request_frame};
use limux_control::socket_path::{bind_listener, resolve_socket_path, SocketMode};
use limux_protocol::{parse_v1_command_envelope, V2Request, V2Response};
use serde_json::{json, Map, Value};

const METHODS: &[&str] = &[
    "system.ping",
    "system.identify",
    "system.capabilities",
    "workspace.current",
    "workspace.list",
    "workspace.create",
    "workspace.select",
    "workspace.rename",
    "workspace.close",
    "surface.send_text",
    "pane.list",
    "pane.surfaces",
    "surface.list",
    "surface.current",
    "browser.open_split",
    "browser.navigate",
    "browser.url.get",
    "browser.back",
    "browser.forward",
    "browser.reload",
    "browser.screenshot",
    "browser.eval",
    "browser.snapshot",
    "browser.click",
    "browser.dblclick",
    "browser.hover",
    "browser.focus",
    "browser.fill",
    "browser.type",
    "browser.press",
    "browser.check",
    "browser.uncheck",
    "browser.select",
    "browser.scroll",
    "browser.scroll_into_view",
    "browser.wait",
    "browser.wait_ready",
    "browser.get.text",
    "browser.get.title",
    "browser.get.html",
    "browser.get.value",
    "browser.get.attr",
    "browser.get.count",
    "browser.get.box",
    "browser.find.role",
    "browser.find.text",
    "browser.find.label",
    "browser.find.placeholder",
    "browser.find.testid",
    "browser.console.list",
    "browser.console.clear",
    "browser.errors.list",
    "browser.errors.clear",
    "browser.is_ready",
    "browser.is_editable",
];

const PARSE_ERROR_CODE: i64 = -32700;
const INVALID_PARAMS_CODE: i64 = -32602;
const UNKNOWN_METHOD_CODE: i64 = -32601;
const INTERNAL_ERROR_CODE: i64 = -32603;
const NOT_FOUND_CODE: i64 = -32004;
const CONFLICT_CODE: i64 = -32009;

type BridgeResult = Result<Value, BridgeError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceTarget {
    Active,
    Handle(String),
    Name(String),
    Index(usize),
}

#[derive(Debug)]
pub enum ControlCommand {
    Identify {
        caller: Option<Value>,
        reply: mpsc::Sender<BridgeResult>,
    },
    CurrentWorkspace {
        reply: mpsc::Sender<BridgeResult>,
    },
    ListWorkspaces {
        reply: mpsc::Sender<BridgeResult>,
    },
    CreateWorkspace {
        name: Option<String>,
        cwd: Option<String>,
        command: Option<String>,
        reply: mpsc::Sender<BridgeResult>,
    },
    SelectWorkspace {
        target: WorkspaceTarget,
        reply: mpsc::Sender<BridgeResult>,
    },
    RenameWorkspace {
        target: WorkspaceTarget,
        title: String,
        reply: mpsc::Sender<BridgeResult>,
    },
    CloseWorkspace {
        target: WorkspaceTarget,
        reply: mpsc::Sender<BridgeResult>,
    },
    SendText {
        target: WorkspaceTarget,
        surface_hint: Option<String>,
        text: String,
        reply: mpsc::Sender<BridgeResult>,
    },
    ListPanes {
        target: WorkspaceTarget,
        reply: mpsc::Sender<BridgeResult>,
    },
    ListSurfaces {
        target: WorkspaceTarget,
        pane_filter: Option<String>,
        reply: mpsc::Sender<BridgeResult>,
    },
    CurrentSurface {
        target: WorkspaceTarget,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserOpenSplit {
        target: WorkspaceTarget,
        source_surface: Option<String>,
        url: Option<String>,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserNavigate {
        surface: String,
        url: String,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserGetUrl {
        surface: String,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserBack {
        surface: String,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserForward {
        surface: String,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserReload {
        surface: String,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserScreenshot {
        surface: String,
        out_path: Option<String>,
        reply: mpsc::Sender<BridgeResult>,
    },
    BrowserEval {
        surface: String,
        script: String,
        /// If Some, the handler parses the JS reply as JSON and wraps it under
        /// this key in the response. If None, the reply is returned verbatim
        /// as a string under the "result" key.
        wrap_key: Option<String>,
        reply: mpsc::Sender<BridgeResult>,
    },
}

impl ControlCommand {
    pub fn respond(self, result: BridgeResult) {
        match self {
            Self::Identify { reply, .. }
            | Self::CurrentWorkspace { reply }
            | Self::ListWorkspaces { reply }
            | Self::CreateWorkspace { reply, .. }
            | Self::SelectWorkspace { reply, .. }
            | Self::RenameWorkspace { reply, .. }
            | Self::CloseWorkspace { reply, .. }
            | Self::SendText { reply, .. }
            | Self::ListPanes { reply, .. }
            | Self::ListSurfaces { reply, .. }
            | Self::CurrentSurface { reply, .. }
            | Self::BrowserOpenSplit { reply, .. }
            | Self::BrowserNavigate { reply, .. }
            | Self::BrowserGetUrl { reply, .. }
            | Self::BrowserBack { reply, .. }
            | Self::BrowserForward { reply, .. }
            | Self::BrowserReload { reply, .. }
            | Self::BrowserScreenshot { reply, .. }
            | Self::BrowserEval { reply, .. } => {
                let _ = reply.send(result);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl BridgeError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS_CODE, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(NOT_FOUND_CODE, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(CONFLICT_CODE, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR_CODE, message)
    }
}

fn parse_request(input: &str) -> Result<V2Request, BridgeError> {
    if let Ok(request) = serde_json::from_str::<V2Request>(input) {
        return Ok(request);
    }

    match parse_v1_command_envelope(input) {
        Ok(v1) => Ok(v1.into_v2_request(None)),
        Err(error) => Err(BridgeError::new(
            PARSE_ERROR_CODE,
            format!("invalid request payload: {error}"),
        )
        .with_data(json!({ "raw": input }))),
    }
}

fn params_object(params: &Value) -> Result<&Map<String, Value>, BridgeError> {
    params
        .as_object()
        .ok_or_else(|| BridgeError::invalid_params("params must be a JSON object"))
}

fn optional_string(params: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        params
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn optional_index(params: &Map<String, Value>, key: &str) -> Result<Option<usize>, BridgeError> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };

    if let Some(index) = value.as_u64() {
        return Ok(Some(index as usize));
    }

    Err(BridgeError::invalid_params(format!(
        "{key} must be a non-negative integer"
    )))
}

fn required_string(
    params: &Map<String, Value>,
    keys: &[&str],
    label: &str,
) -> Result<String, BridgeError> {
    optional_string(params, keys)
        .ok_or_else(|| BridgeError::invalid_params(format!("{label} is required")))
}

/// Strip a leading `prefix:` (e.g. `surface:UUID` → `UUID`) so callers can pass
/// either raw IDs or refs.
fn normalize_handle(raw: String, prefix: &str) -> String {
    raw.strip_prefix(prefix)
        .map(|rest| rest.to_string())
        .unwrap_or(raw)
}

fn parse_optional_workspace_target(
    params: &Map<String, Value>,
    allow_name: bool,
) -> Result<WorkspaceTarget, BridgeError> {
    if let Some(handle) = optional_string(params, &["workspace_id", "id"]) {
        return Ok(WorkspaceTarget::Handle(handle));
    }
    if allow_name {
        if let Some(name) = optional_string(params, &["name"]) {
            return Ok(WorkspaceTarget::Name(name));
        }
    }
    if let Some(index) = optional_index(params, "index")? {
        return Ok(WorkspaceTarget::Index(index));
    }
    Ok(WorkspaceTarget::Active)
}

fn parse_required_workspace_target(
    params: &Map<String, Value>,
    allow_name: bool,
    method: &str,
) -> Result<WorkspaceTarget, BridgeError> {
    let target = parse_optional_workspace_target(params, allow_name)?;
    if matches!(target, WorkspaceTarget::Active) {
        Err(BridgeError::invalid_params(format!(
            "{method} requires workspace_id/id, name, or index"
        )))
    } else {
        Ok(target)
    }
}

fn handle_method(
    id: Option<Value>,
    method: &str,
    params: Value,
    dispatch: &dyn Fn(ControlCommand),
) -> V2Response {
    let params = match params_object(&params) {
        Ok(params) => params,
        Err(error) => return error_response(id, error),
    };

    let queued = match method {
        "system.ping" | "ping" => return V2Response::success(id, json!({ "pong": true })),
        "system.capabilities" => {
            return V2Response::success(id, json!({ "commands": METHODS, "methods": METHODS }));
        }
        "system.identify" => {
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::Identify {
                    caller: params.get("caller").cloned(),
                    reply,
                },
                rx,
            )
        }
        "workspace.current" => {
            let (reply, rx) = mpsc::channel();
            (ControlCommand::CurrentWorkspace { reply }, rx)
        }
        "workspace.list" | "list-workspaces" => {
            let (reply, rx) = mpsc::channel();
            (ControlCommand::ListWorkspaces { reply }, rx)
        }
        "workspace.create" | "new-workspace" => {
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::CreateWorkspace {
                    name: optional_string(params, &["name", "title"]),
                    cwd: optional_string(params, &["cwd"]),
                    command: optional_string(params, &["command"]),
                    reply,
                },
                rx,
            )
        }
        "workspace.select" | "workspace.activate" | "activate-workspace" => {
            let target = match parse_required_workspace_target(params, true, method) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::SelectWorkspace { target, reply }, rx)
        }
        "workspace.rename" | "rename-workspace" => {
            let Some(title) = optional_string(params, &["title", "name"]) else {
                return error_response(
                    id,
                    BridgeError::invalid_params("workspace.rename requires title/name"),
                );
            };
            let target = match parse_optional_workspace_target(params, false) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::RenameWorkspace {
                    target,
                    title,
                    reply,
                },
                rx,
            )
        }
        "workspace.close" | "close-workspace" => {
            let target = match parse_optional_workspace_target(params, false) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::CloseWorkspace { target, reply }, rx)
        }
        "surface.send_text" | "send-text" | "send" => {
            let Some(text) = optional_string(params, &["text"]) else {
                return error_response(
                    id,
                    BridgeError::invalid_params("surface.send_text requires text"),
                );
            };
            let target = match parse_optional_workspace_target(params, false) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::SendText {
                    target,
                    surface_hint: optional_string(params, &["surface_id"]),
                    text,
                    reply,
                },
                rx,
            )
        }
        "pane.list" | "list-panes" | "list-panels" => {
            let target = match parse_optional_workspace_target(params, false) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::ListPanes { target, reply }, rx)
        }
        "pane.surfaces" | "surface.list" => {
            let target = match parse_optional_workspace_target(params, false) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let pane_filter = optional_string(params, &["pane_id", "id"])
                .map(|raw| normalize_handle(raw, "pane:"));
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::ListSurfaces {
                    target,
                    pane_filter,
                    reply,
                },
                rx,
            )
        }
        "surface.current" => {
            let target = match parse_optional_workspace_target(params, false) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::CurrentSurface { target, reply }, rx)
        }
        "browser.open_split" | "browser.open" | "browser.new" => {
            let target = match parse_optional_workspace_target(params, false) {
                Ok(target) => target,
                Err(error) => return error_response(id, error),
            };
            let source_surface = optional_string(params, &["surface_id", "id"])
                .map(|raw| normalize_handle(raw, "surface:"));
            let url = optional_string(params, &["url"]);
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::BrowserOpenSplit {
                    target,
                    source_surface,
                    url,
                    reply,
                },
                rx,
            )
        }
        "browser.navigate" | "browser.goto" => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let url = match required_string(params, &["url"], "url") {
                Ok(value) => value,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::BrowserNavigate {
                    surface,
                    url,
                    reply,
                },
                rx,
            )
        }
        "browser.url.get" | "browser.get.url" => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::BrowserGetUrl { surface, reply }, rx)
        }
        "browser.back" => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::BrowserBack { surface, reply }, rx)
        }
        "browser.forward" => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::BrowserForward { surface, reply }, rx)
        }
        "browser.reload" => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (ControlCommand::BrowserReload { surface, reply }, rx)
        }
        "browser.screenshot" => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let out_path = optional_string(params, &["out", "path"]);
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::BrowserScreenshot {
                    surface,
                    out_path,
                    reply,
                },
                rx,
            )
        }
        "browser.eval" => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let script = match required_string(params, &["script"], "script") {
                Ok(value) => value,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::BrowserEval {
                    surface,
                    script,
                    wrap_key: Some("value".to_string()),
                    reply,
                },
                rx,
            )
        }
        m if m.starts_with("browser.") => {
            let surface = match required_string(params, &["surface_id", "id"], "surface_id") {
                Ok(value) => normalize_handle(value, "surface:"),
                Err(error) => return error_response(id, error),
            };
            let (script, wrap_key) = match build_browser_script(method, params) {
                Ok(pair) => pair,
                Err(error) => return error_response(id, error),
            };
            let (reply, rx) = mpsc::channel();
            (
                ControlCommand::BrowserEval {
                    surface,
                    script,
                    wrap_key,
                    reply,
                },
                rx,
            )
        }
        _ => {
            return error_response(
                id,
                BridgeError::new(UNKNOWN_METHOD_CODE, format!("unknown method: {method}")),
            );
        }
    };

    let (command, reply_rx) = queued;
    let timeout = command_timeout(&command);

    dispatch(command);

    match reply_rx.recv_timeout(timeout) {
        Ok(Ok(result)) => V2Response::success(id, result),
        Ok(Err(error)) => error_response(id, error),
        Err(_) => error_response(id, BridgeError::internal("control command timed out")),
    }
}

fn command_timeout(command: &ControlCommand) -> Duration {
    match command {
        ControlCommand::BrowserEval { .. }
        | ControlCommand::BrowserScreenshot { .. }
        | ControlCommand::BrowserNavigate { .. }
        | ControlCommand::BrowserOpenSplit { .. } => Duration::from_secs(30),
        _ => Duration::from_secs(5),
    }
}

fn error_response(id: Option<Value>, error: BridgeError) -> V2Response {
    V2Response::error(id, error.code, error.message, error.data)
}

fn js_literal(value: &str) -> String {
    serde_json::Value::String(value.to_string()).to_string()
}

fn json_literal(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// JS that resolves a ref-or-selector to a DOM element. When the caller
/// provides a ref (`eN`) the script also verifies attachment and rejects
/// with structured REF_NOT_FOUND / STALE_REF errors rather than silently
/// acting on a stale handle.
fn resolve_target_js(handle_literal: &str) -> String {
    format!(
        r#"(() => {{
            const handle = {h};
            if (!handle) {{ return {{ err: {{ code: "INVALID_PARAMS", message: "selector or ref required" }} }}; }}
            const trimmed = String(handle).replace(/^@/, "");
            const refMatch = /^e\d+$/.test(trimmed);
            if (refMatch) {{
                if (!window.__limux) {{
                    return {{ err: {{ code: "INIT_NOT_READY", message: "limux init script not installed" }} }};
                }}
                const info = window.__limux.refInfo(trimmed);
                if (!info) {{ return {{ err: {{ code: "REF_NOT_FOUND", message: "ref " + trimmed + " unknown", ref: trimmed }} }}; }}
                if (!info.attached) {{ return {{ err: {{ code: "REF_NOT_FOUND", message: "ref " + trimmed + " no longer attached", ref: trimmed }} }}; }}
                const el = window.__limux.lookupRef(trimmed);
                if (!el) {{ return {{ err: {{ code: "REF_NOT_FOUND", message: "ref " + trimmed + " not in DOM", ref: trimmed }} }}; }}
                return {{ el, ref: trimmed, info }};
            }}
            try {{
                const el = document.querySelector(String(handle));
                if (!el) {{ return {{ err: {{ code: "REF_NOT_FOUND", message: "selector matched no element", selector: String(handle) }} }}; }}
                return {{ el, selector: String(handle) }};
            }} catch (e) {{
                return {{ err: {{ code: "INVALID_SELECTOR", message: String(e && e.message || e), selector: String(handle) }} }};
            }}
        }})()"#,
        h = handle_literal
    )
}

/// Wrap a JS action so REF_NOT_FOUND / INVALID_SELECTOR / errors are
/// returned as structured JSON (handler unwraps `err` into BridgeError).
fn wrap_action_js(handle_literal: &str, body: &str) -> String {
    format!(
        r#"(() => {{
            const target = {resolve};
            if (target.err) {{ return JSON.stringify({{ ok: false, error: target.err }}); }}
            const el = target.el;
            try {{
                {body}
            }} catch (e) {{
                return JSON.stringify({{ ok: false, error: {{ code: "INTERNAL", message: String(e && e.message || e) }} }});
            }}
        }})()"#,
        resolve = resolve_target_js(handle_literal),
        body = body,
    )
}

/// Build JS payloads for browser.* methods. All scripts must return a
/// JSON string; the handler parses it and either merges it into the
/// response (when an object + `wrap_key` is None) or stores it under
/// `wrap_key`.
fn build_browser_script(
    method: &str,
    params: &Map<String, Value>,
) -> Result<(String, Option<String>), BridgeError> {
    match method {
        "browser.snapshot" => {
            let opts = snapshot_opts(params);
            let script = crate::pane::LIMUX_BROWSER_SNAPSHOT_SCRIPT
                .replace("__LIMUX_SNAPSHOT_OPTS__", &opts);
            Ok((script, None))
        }
        "browser.click" => Ok((browser_action_click(params)?, None)),
        "browser.dblclick" => Ok((browser_action_dblclick(params)?, None)),
        "browser.hover" => Ok((browser_action_hover(params)?, None)),
        "browser.focus" => Ok((browser_action_focus(params)?, None)),
        "browser.fill" => Ok((browser_action_fill(params)?, None)),
        "browser.type" => Ok((browser_action_type(params)?, None)),
        "browser.press" => Ok((browser_action_press(params)?, None)),
        "browser.check" => Ok((browser_action_toggle_checkable(params, true)?, None)),
        "browser.uncheck" => Ok((browser_action_toggle_checkable(params, false)?, None)),
        "browser.select" => Ok((browser_action_select(params)?, None)),
        "browser.scroll" => Ok((browser_action_scroll(params)?, None)),
        "browser.scroll_into_view" => Ok((browser_action_scroll_into_view(params)?, None)),
        "browser.wait" => Ok((browser_action_wait(params)?, None)),
        "browser.wait_ready" => Ok((browser_action_wait_ready(params)?, None)),
        "browser.get.text" => Ok((browser_get_text(params)?, None)),
        "browser.get.title" => Ok((
            "JSON.stringify({ title: document.title })".to_string(),
            None,
        )),
        "browser.get.html" => Ok((browser_get_html(params)?, None)),
        "browser.get.value" => Ok((browser_get_value(params)?, None)),
        "browser.get.attr" => Ok((browser_get_attr(params)?, None)),
        "browser.get.count" => Ok((browser_get_count(params)?, None)),
        "browser.get.box" => Ok((browser_get_box(params)?, None)),
        "browser.find.role" => Ok((browser_find(params, "role")?, None)),
        "browser.find.text" => Ok((browser_find(params, "text")?, None)),
        "browser.find.label" => Ok((browser_find(params, "label")?, None)),
        "browser.find.placeholder" => Ok((browser_find(params, "placeholder")?, None)),
        "browser.find.testid" => Ok((browser_find(params, "testid")?, None)),
        "browser.console.list" => Ok((browser_console_list(params)?, None)),
        "browser.console.clear" => Ok((
            r#"(() => { if (window.__limux) window.__limux.clearLogs(); return JSON.stringify({ ok: true }); })()"#.to_string(),
            None,
        )),
        "browser.errors.list" => Ok((browser_errors_list(params)?, None)),
        "browser.errors.clear" => Ok((
            r#"(() => { if (window.__limux) window.__limux.clearErrors(); return JSON.stringify({ ok: true }); })()"#.to_string(),
            None,
        )),
        "browser.is_ready" => Ok((
            r#"JSON.stringify({ ready: !!(window.__limux && window.__limux.isReady()) })"#.to_string(),
            None,
        )),
        "browser.is_editable" => Ok((
            r#"JSON.stringify({ editable: !!(window.__limux && window.__limux.isEditable()) })"#.to_string(),
            None,
        )),
        _ => Err(BridgeError::invalid_params(format!(
            "no browser script for {method}"
        ))),
    }
}

fn snapshot_opts(params: &Map<String, Value>) -> String {
    let mut obj = serde_json::Map::new();
    if let Some(v) = params.get("full_tree") {
        obj.insert("full_tree".into(), v.clone());
    }
    if let Some(v) = params.get("raw_html") {
        obj.insert("raw_html".into(), v.clone());
    }
    if let Some(v) = params.get("selector") {
        obj.insert("selector".into(), v.clone());
    }
    if let Some(v) = params.get("max_depth") {
        obj.insert("max_depth".into(), v.clone());
    }
    if let Some(v) = params.get("since_hash") {
        obj.insert("since_hash".into(), v.clone());
    }
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

fn target_handle(params: &Map<String, Value>) -> Result<String, BridgeError> {
    if let Some(s) = optional_string(params, &["ref", "selector", "target"]) {
        return Ok(s);
    }
    Err(BridgeError::invalid_params("ref or selector required"))
}

fn browser_action_click(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = r#"
        if (typeof el.click === "function") { el.click(); }
        else { el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })); }
        return JSON.stringify({ ok: true, ref: target.ref, selector: target.selector });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_action_dblclick(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = r#"
        el.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true }));
        return JSON.stringify({ ok: true, ref: target.ref, selector: target.selector });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_action_hover(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = r#"
        el.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
        el.dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
        return JSON.stringify({ ok: true, ref: target.ref, selector: target.selector });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_action_focus(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = r#"
        if (typeof el.focus === "function") el.focus();
        return JSON.stringify({ ok: true, ref: target.ref, selector: target.selector });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_action_fill(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let text = optional_string(params, &["text", "value"]).unwrap_or_default();
    let body = format!(
        r#"
        const text = {text};
        if (el.isContentEditable) {{
            el.focus();
            el.textContent = text;
            el.dispatchEvent(new Event("input", {{ bubbles: true }}));
        }} else if (el.tagName === "TEXTAREA" || el.tagName === "INPUT" || el.tagName === "SELECT") {{
            const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype
                : el.tagName === "SELECT" ? HTMLSelectElement.prototype
                : HTMLInputElement.prototype;
            const setter = Object.getOwnPropertyDescriptor(proto, "value").set;
            setter.call(el, text);
            el.dispatchEvent(new Event("input", {{ bubbles: true }}));
            el.dispatchEvent(new Event("change", {{ bubbles: true }}));
        }} else {{
            return JSON.stringify({{ ok: false, error: {{ code: "WRONG_ELEMENT", message: "cannot fill tag " + el.tagName }} }});
        }}
        return JSON.stringify({{ ok: true, ref: target.ref, selector: target.selector }});
    "#,
        text = js_literal(&text)
    );
    Ok(wrap_action_js(&js_literal(&handle), &body))
}

fn browser_action_type(params: &Map<String, Value>) -> Result<String, BridgeError> {
    // Dispatch keydown/keypress/keyup + input events for each char on the focused element.
    let text = required_string(params, &["text"], "text")?;
    let script = format!(
        r#"(() => {{
            const text = {text};
            const el = document.activeElement;
            if (!el) {{ return JSON.stringify({{ ok: false, error: {{ code: "NO_FOCUS", message: "no focused element" }} }}); }}
            for (const ch of text) {{
                el.dispatchEvent(new KeyboardEvent("keydown", {{ key: ch, bubbles: true }}));
                el.dispatchEvent(new KeyboardEvent("keypress", {{ key: ch, bubbles: true }}));
                if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {{
                    const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
                    const setter = Object.getOwnPropertyDescriptor(proto, "value").set;
                    setter.call(el, (el.value || "") + ch);
                    el.dispatchEvent(new Event("input", {{ bubbles: true }}));
                }} else if (el.isContentEditable) {{
                    el.textContent = (el.textContent || "") + ch;
                    el.dispatchEvent(new Event("input", {{ bubbles: true }}));
                }}
                el.dispatchEvent(new KeyboardEvent("keyup", {{ key: ch, bubbles: true }}));
            }}
            return JSON.stringify({{ ok: true }});
        }})()"#,
        text = js_literal(&text)
    );
    Ok(script)
}

fn browser_action_press(params: &Map<String, Value>) -> Result<String, BridgeError> {
    // keys = "Enter", "Ctrl+K", etc. Parse into key + modifier flags.
    let keys = required_string(params, &["keys", "key"], "keys")?;
    let script = format!(
        r#"(() => {{
            const raw = {keys};
            const parts = String(raw).split("+").map(s => s.trim());
            const key = parts.pop();
            const mods = new Set(parts.map(s => s.toLowerCase()));
            const el = document.activeElement || document.body;
            const init = {{
                key,
                bubbles: true,
                cancelable: true,
                ctrlKey: mods.has("ctrl") || mods.has("control"),
                altKey: mods.has("alt"),
                shiftKey: mods.has("shift"),
                metaKey: mods.has("meta") || mods.has("cmd") || mods.has("super"),
            }};
            el.dispatchEvent(new KeyboardEvent("keydown", init));
            el.dispatchEvent(new KeyboardEvent("keypress", init));
            el.dispatchEvent(new KeyboardEvent("keyup", init));
            return JSON.stringify({{ ok: true, key, mods: [...mods] }});
        }})()"#,
        keys = js_literal(&keys)
    );
    Ok(script)
}

fn browser_action_toggle_checkable(
    params: &Map<String, Value>,
    desired: bool,
) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = format!(
        r#"
        if (el.type !== "checkbox" && el.type !== "radio" && el.getAttribute("role") !== "checkbox" && el.getAttribute("role") !== "switch") {{
            return JSON.stringify({{ ok: false, error: {{ code: "WRONG_ELEMENT", message: "not a checkable element" }} }});
        }}
        const want = {desired};
        if (el.type === "radio") {{
            if (want) {{ if (!el.checked) el.click(); }}
            else {{ return JSON.stringify({{ ok: false, error: {{ code: "INVALID_OP", message: "cannot uncheck a radio" }} }}); }}
        }} else if (el.checked !== want) {{
            el.click();
        }}
        return JSON.stringify({{ ok: true, ref: target.ref, selector: target.selector, checked: !!el.checked }});
    "#,
        desired = desired
    );
    Ok(wrap_action_js(&js_literal(&handle), &body))
}

fn browser_action_select(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let option = required_string(params, &["option", "value", "label"], "option")?;
    let body = format!(
        r#"
        if (el.tagName !== "SELECT") {{ return JSON.stringify({{ ok: false, error: {{ code: "WRONG_ELEMENT", message: "not a <select>" }} }}); }}
        const want = {opt};
        let matched = false;
        for (const option of el.options) {{
            if (option.value === want || option.label === want || option.text === want) {{
                el.value = option.value;
                matched = true;
                break;
            }}
        }}
        if (!matched) {{ return JSON.stringify({{ ok: false, error: {{ code: "OPTION_NOT_FOUND", message: "option not found", option: want }} }}); }}
        el.dispatchEvent(new Event("input", {{ bubbles: true }}));
        el.dispatchEvent(new Event("change", {{ bubbles: true }}));
        return JSON.stringify({{ ok: true, ref: target.ref, selector: target.selector, value: el.value }});
    "#,
        opt = js_literal(&option)
    );
    Ok(wrap_action_js(&js_literal(&handle), &body))
}

fn browser_action_scroll(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let x = params.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let y = params.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let script = format!(
        r#"(() => {{
            window.scrollBy({{ left: {x}, top: {y}, behavior: "instant" }});
            return JSON.stringify({{ ok: true, x: window.scrollX, y: window.scrollY }});
        }})()"#
    );
    Ok(script)
}

fn browser_action_scroll_into_view(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = r#"
        if (typeof el.scrollIntoView === "function") el.scrollIntoView({ behavior: "instant", block: "center", inline: "center" });
        return JSON.stringify({ ok: true, ref: target.ref, selector: target.selector });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_action_wait(params: &Map<String, Value>) -> Result<String, BridgeError> {
    // Poll every 100ms up to timeout_ms (default 5000). Condition: either
    // selector matches OR ref is attached OR document ready flag.
    let selector = optional_string(params, &["selector"]);
    let ref_id = optional_string(params, &["ref"]);
    let timeout_ms = params
        .get("timeout_ms")
        .or_else(|| params.get("timeout"))
        .and_then(|v| v.as_u64())
        .unwrap_or(5000);
    let ready_flag = params
        .get("ready")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let script = format!(
        r#"(async () => {{
            const selector = {sel};
            const refId = {rf};
            const readyFlag = {ready};
            const deadline = performance.now() + {timeout};
            while (performance.now() < deadline) {{
                if (readyFlag && window.__limux && window.__limux.isReady()) {{
                    return JSON.stringify({{ ok: true, reason: "ready" }});
                }}
                if (selector) {{
                    try {{
                        const el = document.querySelector(selector);
                        if (el) return JSON.stringify({{ ok: true, selector, reason: "selector" }});
                    }} catch (e) {{
                        return JSON.stringify({{ ok: false, error: {{ code: "INVALID_SELECTOR", message: String(e && e.message || e) }} }});
                    }}
                }}
                if (refId) {{
                    const info = window.__limux && window.__limux.refInfo(refId);
                    if (info && info.attached) return JSON.stringify({{ ok: true, ref: refId, reason: "ref" }});
                }}
                await new Promise(r => setTimeout(r, 100));
            }}
            return JSON.stringify({{ ok: false, error: {{ code: "TIMEOUT", message: "wait timed out", timeout_ms: {timeout} }} }});
        }})()"#,
        sel = json_literal(&Value::from(selector)),
        rf = json_literal(&Value::from(ref_id)),
        ready = ready_flag,
        timeout = timeout_ms
    );
    Ok(script)
}

fn browser_action_wait_ready(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let timeout_ms = params
        .get("timeout_ms")
        .or_else(|| params.get("timeout"))
        .and_then(|v| v.as_u64())
        .unwrap_or(30000);
    let script = format!(
        r#"(async () => {{
            const deadline = performance.now() + {timeout};
            while (performance.now() < deadline) {{
                if (window.__limux && window.__limux.isReady()) {{
                    return JSON.stringify({{ ok: true }});
                }}
                await new Promise(r => setTimeout(r, 100));
            }}
            return JSON.stringify({{ ok: false, error: {{ code: "TIMEOUT", message: "page not ready within " + {timeout} + "ms" }} }});
        }})()"#,
        timeout = timeout_ms
    );
    Ok(script)
}

fn browser_get_text(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = optional_string(params, &["ref", "selector", "target"])
        .unwrap_or_else(|| "body".to_string());
    let body = r#"
        const text = el.innerText || el.textContent || "";
        return JSON.stringify({ ok: true, text });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_get_html(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = optional_string(params, &["ref", "selector", "target"]);
    match handle {
        Some(h) => {
            let body = r#"
                return JSON.stringify({ ok: true, html: el.outerHTML });
            "#;
            Ok(wrap_action_js(&js_literal(&h), body))
        }
        None => Ok(
            r#"JSON.stringify({ ok: true, html: document.documentElement.outerHTML })"#.to_string(),
        ),
    }
}

fn browser_get_value(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = r#"
        return JSON.stringify({ ok: true, value: el.value ?? null });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_get_attr(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let name = required_string(params, &["attr", "name"], "attr")?;
    let body = format!(
        r#"
        return JSON.stringify({{ ok: true, value: el.getAttribute({n}) }});
    "#,
        n = js_literal(&name)
    );
    Ok(wrap_action_js(&js_literal(&handle), &body))
}

fn browser_get_count(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let selector = required_string(params, &["selector"], "selector")?;
    let script = format!(
        r#"(() => {{
            try {{
                return JSON.stringify({{ ok: true, count: document.querySelectorAll({s}).length }});
            }} catch (e) {{
                return JSON.stringify({{ ok: false, error: {{ code: "INVALID_SELECTOR", message: String(e && e.message || e) }} }});
            }}
        }})()"#,
        s = js_literal(&selector)
    );
    Ok(script)
}

fn browser_get_box(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let handle = target_handle(params)?;
    let body = r#"
        const r = el.getBoundingClientRect();
        return JSON.stringify({ ok: true, box: { x: r.left, y: r.top, w: r.width, h: r.height } });
    "#;
    Ok(wrap_action_js(&js_literal(&handle), body))
}

fn browser_find(params: &Map<String, Value>, kind: &str) -> Result<String, BridgeError> {
    let value = required_string(
        params,
        &[
            "value",
            "name",
            "role",
            "text",
            "label",
            "placeholder",
            "testid",
        ],
        "value",
    )?;
    let script = format!(
        r#"(() => {{
            if (!window.__limux) return JSON.stringify({{ ok: false, error: {{ code: "INIT_NOT_READY", message: "init script not installed" }} }});
            const kind = {kind};
            const want = {v};
            let el = null;
            if (kind === "role") {{
                const nodes = document.querySelectorAll("[role], a, button, input, select, textarea, summary, details");
                for (const node of nodes) {{
                    if (window.__limux.elementRole(node) === want) {{ el = node; break; }}
                }}
            }} else if (kind === "text") {{
                // Prefer interactive elements with exact-text match so we
                // don't accidentally target an ancestor wrapper.
                const interactive = document.querySelectorAll(
                    "a[href], button, input, select, textarea, summary, " +
                    "[role=button], [role=link], [role=checkbox], [role=radio], " +
                    "[role=menuitem], [role=tab], [role=option]"
                );
                for (const node of interactive) {{
                    const label = ((node.innerText || node.textContent || "").trim()) || node.value || node.getAttribute("aria-label") || "";
                    if (label === want) {{ el = node; break; }}
                }}
                if (!el) {{
                    const all = document.querySelectorAll("h1, h2, h3, h4, h5, h6, label, [role=heading], [role=listitem], [role=alert]");
                    for (const node of all) {{
                        if ((node.innerText || node.textContent || "").trim() === want) {{ el = node; break; }}
                    }}
                }}
            }} else if (kind === "label") {{
                const label = document.querySelector("label");
                const all = document.querySelectorAll("label");
                for (const lbl of all) {{
                    if ((lbl.textContent || "").trim() === want) {{
                        if (lbl.htmlFor) el = document.getElementById(lbl.htmlFor);
                        if (!el) el = lbl.querySelector("input, textarea, select");
                        if (el) break;
                    }}
                }}
            }} else if (kind === "placeholder") {{
                el = document.querySelector('[placeholder="' + CSS.escape(want) + '"]');
            }} else if (kind === "testid") {{
                el = document.querySelector('[data-testid="' + CSS.escape(want) + '"]');
                if (!el) el = document.querySelector('[data-test-id="' + CSS.escape(want) + '"]');
            }}
            if (!el) return JSON.stringify({{ ok: false, error: {{ code: "NOT_FOUND", message: "find." + kind + " matched nothing", query: want }} }});
            const id = window.__limux.assignRef(el);
            return JSON.stringify({{ ok: true, ref: id, role: window.__limux.elementRole(el), name: window.__limux.accessibleName(el) }});
        }})()"#,
        kind = js_literal(kind),
        v = js_literal(&value)
    );
    Ok(script)
}

fn browser_console_list(params: &Map<String, Value>) -> Result<String, BridgeError> {
    // `since` filters by monotonic seq number (returned in each entry). The
    // caller records the largest `seq` it saw and passes it back next time.
    // ts_ms is informational only — webkit6 clamps wall-clock to i32::MIN so
    // seq is the only reliable ordering.
    let since = params.get("since").and_then(|v| v.as_u64()).unwrap_or(0);
    let clear_after = params
        .get("clear_after")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let levels = optional_string(params, &["level", "levels"]);
    let levels_filter = levels
        .map(|s| {
            let v: Vec<String> = s.split(',').map(|x| x.trim().to_string()).collect();
            format!(
                "[{}]",
                v.iter()
                    .map(|x| js_literal(x))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .unwrap_or_else(|| "null".to_string());
    let script = format!(
        r#"(() => {{
            if (!window.__limux) return JSON.stringify({{ logs: [], dropped: 0 }});
            const since = {since};
            const levels = {levels};
            const logs = window.__limux.logs.filter(e => e.seq > since && (!levels || levels.includes(e.level)));
            const dropped = window.__limux.logsDroppedCount;
            const latest = logs.length ? logs[logs.length - 1].seq : since;
            if ({clear}) window.__limux.clearLogs();
            return JSON.stringify({{ logs, dropped, latest_seq: latest }});
        }})()"#,
        since = since,
        levels = levels_filter,
        clear = clear_after
    );
    Ok(script)
}

fn browser_errors_list(params: &Map<String, Value>) -> Result<String, BridgeError> {
    let since = params.get("since").and_then(|v| v.as_u64()).unwrap_or(0);
    let clear_after = params
        .get("clear_after")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let script = format!(
        r#"(() => {{
            if (!window.__limux) return JSON.stringify({{ errors: [], dropped: 0 }});
            const since = {since};
            const errors = window.__limux.errors.filter(e => e.seq > since);
            const dropped = window.__limux.errorsDroppedCount;
            const latest = errors.length ? errors[errors.length - 1].seq : since;
            if ({clear}) window.__limux.clearErrors();
            return JSON.stringify({{ errors, dropped, latest_seq: latest }});
        }})()"#,
        since = since,
        clear = clear_after
    );
    Ok(script)
}

fn dispatch_request(input: &str, dispatch: &dyn Fn(ControlCommand)) -> V2Response {
    match parse_request(input) {
        Ok(request) => handle_method(request.id, &request.method, request.params, dispatch),
        Err(error) => error_response(None, error),
    }
}

fn handle_client(
    stream: UnixStream,
    dispatch: &(dyn Fn(ControlCommand) + Send + Sync + 'static),
) -> io::Result<()> {
    stream.set_read_timeout(Some(request_io::CLIENT_IDLE_TIMEOUT))?;
    let reader_stream = stream.try_clone()?;
    reader_stream.set_read_timeout(Some(request_io::CLIENT_IDLE_TIMEOUT))?;
    let mut reader = io::BufReader::new(reader_stream);
    let mut writer = stream;
    let mut line_buf = Vec::with_capacity(4096);

    loop {
        if !read_request_frame(&mut reader, &mut line_buf)? {
            return Ok(());
        }

        let input = std::str::from_utf8(&line_buf)
            .map(|line| line.trim_end_matches(['\n', '\r']))
            .unwrap_or("");
        if input.is_empty() {
            continue;
        }

        let response = dispatch_request(input, dispatch);
        let mut payload = serde_json::to_string(&response)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        payload.push('\n');
        writer.write_all(payload.as_bytes())?;
        writer.flush()?;
    }
}

struct ConnectionSlot {
    active_connections: Arc<AtomicUsize>,
}

impl ConnectionSlot {
    fn try_acquire(active_connections: Arc<AtomicUsize>) -> Option<Self> {
        active_connections
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < request_io::MAX_CONNECTIONS).then_some(current + 1)
            })
            .ok()?;
        Some(Self { active_connections })
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.active_connections.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Start the control socket server in a background thread and dispatch each
/// command onto the GTK main context.
pub fn start(dispatch: fn(ControlCommand)) {
    let context = glib::MainContext::default();
    let dispatch = std::sync::Arc::new(move |command: ControlCommand| {
        context.invoke(move || dispatch(command));
    });

    std::thread::Builder::new()
        .name("limux-control".into())
        .spawn(move || {
            let path = resolve_socket_path(None, SocketMode::Runtime);
            let control_mode = SocketControlMode::from_env();
            let listener = match bind_listener(
                &path,
                SocketMode::Runtime,
                control_mode.requires_owner_only_socket(),
            ) {
                Ok(listener) => listener,
                Err(error) => {
                    eprintln!(
                        "limux: control socket bind failed ({}): {error}",
                        path.display()
                    );
                    return;
                }
            };

            eprintln!("limux: control socket at {}", path.display());
            let active_connections = Arc::new(AtomicUsize::new(0));

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let Some(slot) = ConnectionSlot::try_acquire(active_connections.clone()) else {
                            eprintln!("limux: rejecting control client, too many active connections");
                            continue;
                        };
                        let peer = match auth::authorize_peer(&stream, control_mode) {
                            Ok(peer) => peer,
                            Err(error) => {
                                eprintln!("limux: rejected control client: {error}");
                                continue;
                            }
                        };
                        let dispatch = dispatch.clone();
                        std::thread::Builder::new()
                            .name("limux-ctrl-conn".into())
                            .spawn(move || {
                                let _slot = slot;
                                if let Err(error) = handle_client(stream, dispatch.as_ref()) {
                                    eprintln!(
                                        "limux: control connection error for pid={} uid={}: {error}",
                                        peer.pid, peer.uid
                                    );
                                }
                            })
                            .ok();
                    }
                    Err(error) => {
                        eprintln!("limux: control accept error: {error}");
                    }
                }
            }
        })
        .expect("failed to spawn control server thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_request_directly() {
        let request = parse_request(r#"{"id":"1","method":"system.ping","params":{}}"#)
            .expect("v2 request should parse");
        assert_eq!(request.id, Some(Value::String("1".to_string())));
        assert_eq!(request.method, "system.ping");
    }

    #[test]
    fn parses_v1_request_envelope() {
        let request = parse_request(r#"{"command":"workspace.create","args":{"cwd":"/tmp"}}"#)
            .expect("v1 request should parse");
        assert_eq!(request.method, "workspace.create");
        assert_eq!(request.params["cwd"], "/tmp");
    }

    #[test]
    fn workspace_target_prefers_handle_over_index() {
        let params = json!({
            "workspace_id": "workspace:abc",
            "index": 2
        });
        let target =
            parse_optional_workspace_target(params.as_object().expect("object params"), true)
                .expect("target should parse");
        assert_eq!(target, WorkspaceTarget::Handle("workspace:abc".to_string()));
    }

    #[test]
    fn workspace_select_requires_explicit_target() {
        let params = Map::new();
        let error = parse_required_workspace_target(&params, true, "workspace.select")
            .expect_err("workspace.select should require a target");
        assert_eq!(error.code, INVALID_PARAMS_CODE);
    }
}
