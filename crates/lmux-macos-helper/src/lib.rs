#[cfg(target_os = "macos")]
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::ffi::CStr;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(all(target_os = "macos", test))]
use std::sync::Mutex;
use std::{
    io::{BufRead, Write},
    process::Command,
    time::Instant,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<i64>,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    pub window_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPreview {
    pub width: u32,
    pub height: u32,
    pub bytes_per_row: usize,
    pub bgra: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    pub window: WindowRef,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum HelperRequest {
    Health,
    ListWindows {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bundle_id: Option<String>,
    },
    FocusedWindow,
    SetVisible {
        window: WindowRef,
        visible: bool,
    },
    ApplyGroup {
        hide: Vec<WindowRef>,
        show: Vec<WindowRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HelperResponse {
    Ok,
    Health {
        backend: String,
    },
    Windows {
        windows: Vec<WindowInfo>,
    },
    FocusedWindow {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window: Option<WindowInfo>,
    },
    Applied {
        results: Vec<OperationResult>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum HelperError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("coregraphics failed: {0}")]
    CoreGraphics(String),
    #[error("osascript failed: {0}")]
    Osascript(String),
}

#[cfg(all(target_os = "macos", test))]
static TEST_WINDOW_ID_VISIBILITY_RESULT: Mutex<Option<bool>> = Mutex::new(None);
#[cfg(test)]
static TEST_OSASCRIPT_CALLS: AtomicUsize = AtomicUsize::new(0);

pub fn run_stdio<R, W>(reader: R, mut writer: W) -> anyhow::Result<()>
where
    R: std::io::Read,
    W: Write,
{
    let reader = std::io::BufReader::new(reader);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<HelperRequest>(&line) {
            Ok(request) => handle_request(request),
            Err(err) => HelperResponse::Error {
                message: format!("invalid request: {err}"),
            },
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

pub fn handle_request(request: HelperRequest) -> HelperResponse {
    match request {
        HelperRequest::Health => HelperResponse::Health {
            backend: "macos-accessibility".into(),
        },
        HelperRequest::ListWindows { pid, bundle_id } => match list_windows(pid, bundle_id) {
            Ok(windows) => HelperResponse::Windows { windows },
            Err(err) => HelperResponse::Error {
                message: err.to_string(),
            },
        },
        HelperRequest::FocusedWindow => match focused_window() {
            Ok(window) => HelperResponse::FocusedWindow { window },
            Err(err) => HelperResponse::Error {
                message: err.to_string(),
            },
        },
        HelperRequest::SetVisible { window, visible } => {
            let result = set_visible(&window, visible);
            if let Err(err) = result {
                return HelperResponse::Error {
                    message: err.to_string(),
                };
            }
            HelperResponse::Ok
        }
        HelperRequest::ApplyGroup { hide, show } => {
            let started = Instant::now();
            let hide_count = hide.len();
            let show_count = show.len();
            let results = apply_group(hide, show);
            let failures = results.iter().filter(|result| !result.ok).count();
            tracing::debug!(
                operation = "macos.helper.apply_group",
                duration_ms = elapsed_ms(started),
                hide = hide_count,
                show = show_count,
                windows = results.len(),
                failures,
                "macOS helper apply group finished"
            );
            HelperResponse::Applied { results }
        }
    }
}

fn apply_group(hide: Vec<WindowRef>, show: Vec<WindowRef>) -> Vec<OperationResult> {
    let mut results = Vec::with_capacity(hide.len() + show.len());
    for window in hide {
        results.push(operation_result(window, false));
    }
    for window in show {
        results.push(operation_result_fast_show(&window));
    }
    results
}

fn operation_result(window: WindowRef, visible: bool) -> OperationResult {
    match set_visible(&window, visible) {
        Ok(()) => OperationResult {
            window,
            ok: true,
            error: None,
        },
        Err(err) => OperationResult {
            window,
            ok: false,
            error: Some(err.to_string()),
        },
    }
}

fn operation_result_fast_show(window: &WindowRef) -> OperationResult {
    #[cfg(target_os = "macos")]
    if window.window_id.is_some() {
        match set_visible_by_window_id(window, true) {
            Ok(true) => {
                return OperationResult {
                    window: window.clone(),
                    ok: true,
                    error: None,
                };
            }
            Ok(false) => {}
            Err(err) => {
                return OperationResult {
                    window: window.clone(),
                    ok: false,
                    error: Some(err.to_string()),
                };
            }
        }
    }
    operation_result(window.clone(), true)
}

pub fn list_windows(
    pid: Option<u32>,
    bundle_id: Option<String>,
) -> Result<Vec<WindowInfo>, HelperError> {
    let started = Instant::now();
    let result = list_windows_inner(pid, bundle_id);
    tracing::debug!(
        operation = "macos.helper.list_windows",
        duration_ms = elapsed_ms(started),
        windows = result.as_ref().map(|windows| windows.len()).unwrap_or(0),
        ok = result.is_ok(),
        error = result.as_ref().err().map(ToString::to_string),
        "macOS helper list windows finished"
    );
    result
}

fn list_windows_inner(
    pid: Option<u32>,
    bundle_id: Option<String>,
) -> Result<Vec<WindowInfo>, HelperError> {
    #[cfg(target_os = "macos")]
    {
        return list_windows_accessibility(pid, bundle_id.as_deref());
    }

    #[cfg(not(target_os = "macos"))]
    list_windows_system_events(pid, bundle_id.as_deref())
}

pub fn focused_window() -> Result<Option<WindowInfo>, HelperError> {
    let script = r#"tell application "System Events"
    set matches to application processes whose frontmost is true
    if (count of matches) is 0 then return ""
    set p to item 1 of matches
    set processPid to 0
    set processBundle to ""
    try
        set processPid to unix id of p
    end try
    try
        set processBundle to bundle identifier of p
    end try
    if processPid is 0 then return ""
    if (count of windows of p) is 0 then return (processPid as text) & tab & processBundle & tab & "0" & tab & "" & tab & "0"
    set w to window 1 of p
    set windowTitle to ""
    set windowNumber to 0
    try
        set windowTitle to name of w
    end try
    try
        set windowNumber to value of attribute "AXWindowNumber" of w
    end try
    return (processPid as text) & tab & processBundle & tab & "1" & tab & windowTitle & tab & (windowNumber as text)
end tell"#;
    let output = run_osascript(script)?;
    let line = output.trim_end();
    if line.is_empty() {
        return Ok(None);
    }
    let mut parts = line.splitn(5, '\t');
    let Some(pid) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
        return Ok(None);
    };
    let bundle_id = parts.next().and_then(non_empty_string);
    let window_index = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let title = parts.next().and_then(non_empty_string);
    let window_id = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0);

    if window_id.is_some() {
        return Ok(Some(WindowInfo {
            window_id,
            pid,
            bundle_id,
            window_index,
            title,
        }));
    }

    let current = list_windows(Some(pid), None)?;
    let exact = current.iter().find(|window| {
        window.window_index == window_index
            && (title.is_none() || window.title.as_deref() == title.as_deref())
    });
    if let Some(window) = exact.or_else(|| current.first()) {
        let mut window = window.clone();
        if window.bundle_id.is_none() {
            window.bundle_id = bundle_id;
        }
        return Ok(Some(window));
    }
    Ok(Some(WindowInfo {
        window_id: None,
        pid,
        bundle_id,
        window_index,
        title,
    }))
}

pub fn window_preview(
    window: &WindowInfo,
    max_width: u32,
    max_height: u32,
) -> Result<Option<WindowPreview>, HelperError> {
    #[cfg(target_os = "macos")]
    {
        return window_preview_inner(window, max_width, max_height);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, max_width, max_height);
        Ok(None)
    }
}

#[cfg(not(target_os = "macos"))]
fn list_windows_system_events(
    pid: Option<u32>,
    bundle_id: Option<&str>,
) -> Result<Vec<WindowInfo>, HelperError> {
    let pid_filter = pid.unwrap_or(0);
    let bundle_filter = apple_script_string(bundle_id.unwrap_or(""));
    let script = format!(
        r#"tell application "System Events"
    set out to ""
    set pidFilter to {pid_filter}
    set bundleFilter to {bundle_filter}
    repeat with p in application processes
        set processPid to 0
        set processBundle to ""
        try
            set processPid to unix id of p
        end try
        try
            set processBundle to bundle identifier of p
        end try
        if (pidFilter is 0 or processPid is pidFilter) and (bundleFilter is "" or processBundle is bundleFilter) then
            set windowIndex to 1
            repeat with w in windows of p
                set windowTitle to ""
                try
                    set windowTitle to name of w
                end try
                set windowNumber to 0
                try
                    set windowNumber to value of attribute "AXWindowNumber" of w
                end try
                set out to out & (processPid as text) & tab & processBundle & tab & (windowIndex as text) & tab & windowTitle & tab & (windowNumber as text) & linefeed
                set windowIndex to windowIndex + 1
            end repeat
        end if
    end repeat
    return out
end tell"#
    );
    let output = run_osascript(&script)?;
    let mut windows = Vec::new();
    for line in output.lines() {
        let mut parts = line.splitn(5, '\t');
        let Some(pid) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let bundle_id = parts.next().and_then(non_empty_string);
        let Some(window_index) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let title = parts.next().and_then(non_empty_string);
        let window_id = parts
            .next()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0);
        windows.push(WindowInfo {
            window_id,
            pid,
            bundle_id,
            window_index,
            title,
        });
    }
    Ok(windows)
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppProcess {
    pid: u32,
    bundle_id: Option<String>,
}

#[cfg(target_os = "macos")]
fn list_windows_accessibility(
    pid_filter: Option<u32>,
    bundle_filter: Option<&str>,
) -> Result<Vec<WindowInfo>, HelperError> {
    let processes = match pid_filter {
        Some(pid) => vec![AppProcess {
            pid,
            bundle_id: bundle_filter.map(str::to_owned),
        }],
        None => match application_processes_system_events(bundle_filter) {
            Ok(processes) if !processes.is_empty() => processes,
            Ok(_) | Err(_) if bundle_filter.is_none() => visible_window_pids()?
                .into_iter()
                .map(|pid| AppProcess {
                    pid,
                    bundle_id: None,
                })
                .collect(),
            Ok(_) => Vec::new(),
            Err(err) => return Err(err),
        },
    };
    let bundle_by_pid: HashMap<u32, Option<String>> = processes
        .iter()
        .map(|process| (process.pid, process.bundle_id.clone()))
        .collect();
    let windows_attr = create_cf_string(c"AXWindows")?;
    let title_attr = create_cf_string(c"AXTitle")?;
    let window_number_attr = create_cf_string(c"AXWindowNumber")?;
    let position_attr = create_cf_string(c"AXPosition")?;
    let size_attr = create_cf_string(c"AXSize")?;
    let mut windows = Vec::new();

    for process in processes {
        let pid = process.pid;
        let app = unsafe { AXUIElementCreateApplication(pid as libc::pid_t) };
        if app.is_null() {
            continue;
        }
        let mut windows_value = std::ptr::null();
        let copy_error =
            unsafe { AXUIElementCopyAttributeValue(app, windows_attr, &mut windows_value) };
        if copy_error != K_AX_ERROR_SUCCESS || windows_value.is_null() {
            unsafe { CFRelease(app as CFTypeRef) };
            continue;
        }

        let ax_windows = windows_value as CFArrayRef;
        let count = unsafe { CFArrayGetCount(ax_windows) };
        let cg_candidates = cg_window_candidates_for_pid(pid).unwrap_or_default();
        for index in 0..count {
            let ax_window = unsafe { CFArrayGetValueAtIndex(ax_windows, index) as AXUIElementRef };
            if ax_window.is_null() {
                continue;
            }
            let title = ax_attribute_string(ax_window, title_attr);
            let position = ax_attribute_point(ax_window, position_attr);
            let size = ax_attribute_size(ax_window, size_attr);
            let window_id = ax_cg_window_id(ax_window)
                .or_else(|| ax_attribute_i64(ax_window, window_number_attr))
                .or_else(|| {
                    cg_window_id_for_ax_window(&cg_candidates, title.as_deref(), position, size)
                });
            windows.push(WindowInfo {
                window_id,
                pid,
                bundle_id: bundle_by_pid.get(&pid).cloned().flatten(),
                window_index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                title,
            });
        }
        unsafe {
            CFRelease(windows_value);
            CFRelease(app as CFTypeRef);
        }
    }

    unsafe {
        CFRelease(windows_attr as CFTypeRef);
        CFRelease(title_attr as CFTypeRef);
        CFRelease(window_number_attr as CFTypeRef);
        CFRelease(position_attr as CFTypeRef);
        CFRelease(size_attr as CFTypeRef);
    }
    Ok(windows)
}

#[cfg(target_os = "macos")]
fn application_processes_system_events(
    bundle_filter: Option<&str>,
) -> Result<Vec<AppProcess>, HelperError> {
    let bundle_filter = apple_script_string(bundle_filter.unwrap_or(""));
    let script = format!(
        r#"tell application "System Events"
    set out to ""
    set bundleFilter to {bundle_filter}
    repeat with p in application processes
        set processPid to 0
        set processBundle to ""
        try
            set processPid to unix id of p
        end try
        try
            set processBundle to bundle identifier of p
        end try
        if processPid is not 0 and (bundleFilter is "" or processBundle is bundleFilter) then
            set out to out & (processPid as text) & tab & processBundle & linefeed
        end if
    end repeat
    return out
end tell"#
    );
    parse_application_processes(&run_osascript(&script)?)
}

#[cfg(target_os = "macos")]
fn parse_application_processes(output: &str) -> Result<Vec<AppProcess>, HelperError> {
    let mut processes = Vec::new();
    for line in output.lines() {
        let mut parts = line.splitn(2, '\t');
        let Some(pid) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let bundle_id = parts.next().and_then(non_empty_string);
        if processes
            .iter()
            .any(|existing: &AppProcess| existing.pid == pid)
        {
            continue;
        }
        processes.push(AppProcess { pid, bundle_id });
    }
    Ok(processes)
}

#[cfg(target_os = "macos")]
fn visible_window_pids() -> Result<Vec<u32>, HelperError> {
    let array = unsafe { CGWindowListCopyWindowInfo(K_CG_WINDOW_LIST_OPTION_ALL, 0) };
    if array.is_null() {
        return Err(HelperError::CoreGraphics(
            "CGWindowListCopyWindowInfo returned null".into(),
        ));
    }

    let mut pids = Vec::new();
    unsafe {
        let count = CFArrayGetCount(array);
        for index in 0..count {
            let dict = CFArrayGetValueAtIndex(array, index) as CFDictionaryRef;
            if dict.is_null() {
                continue;
            }
            if cf_dictionary_i64(dict, kCGWindowLayer).unwrap_or(0) != 0 {
                continue;
            }
            let Some(pid) =
                cf_dictionary_i64(dict, kCGWindowOwnerPID).and_then(|pid| u32::try_from(pid).ok())
            else {
                continue;
            };
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        CFRelease(array as CFTypeRef);
    }
    Ok(pids)
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct CgWindowCandidate {
    window_id: u32,
    title: Option<String>,
    bounds: Option<CgWindowBounds>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct CgWindowBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
fn window_preview_inner(
    window: &WindowInfo,
    max_width: u32,
    max_height: u32,
) -> Result<Option<WindowPreview>, HelperError> {
    let Some(window_id) = cg_window_id_for_window(window)? else {
        return Ok(None);
    };
    capture_cg_window(window_id, max_width, max_height)
}

#[cfg(target_os = "macos")]
fn cg_window_id_for_window(window: &WindowInfo) -> Result<Option<u32>, HelperError> {
    if let Some(window_id) = window.window_id.and_then(|id| u32::try_from(id).ok()) {
        return Ok(Some(window_id));
    }

    let candidates = cg_window_candidates_for_pid(window.pid)?;
    if candidates.is_empty() {
        return Ok(None);
    }

    let by_index = window
        .window_index
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| candidates.get(index));
    if let Some(candidate) = by_index {
        if window.title.is_none() || candidate.title.as_deref() == window.title.as_deref() {
            return Ok(Some(candidate.window_id));
        }
    }

    Ok(window.title.as_deref().and_then(|title| {
        candidates
            .iter()
            .find(|candidate| candidate.title.as_deref() == Some(title))
            .map(|candidate| candidate.window_id)
    }))
}

#[cfg(target_os = "macos")]
fn cg_window_candidates_for_pid(pid_filter: u32) -> Result<Vec<CgWindowCandidate>, HelperError> {
    let array = unsafe { CGWindowListCopyWindowInfo(K_CG_WINDOW_LIST_OPTION_ALL, 0) };
    if array.is_null() {
        return Err(HelperError::CoreGraphics(
            "CGWindowListCopyWindowInfo returned null".into(),
        ));
    }

    let mut candidates = Vec::new();
    unsafe {
        let count = CFArrayGetCount(array);
        for index in 0..count {
            let dict = CFArrayGetValueAtIndex(array, index) as CFDictionaryRef;
            if dict.is_null() {
                continue;
            }
            if cf_dictionary_i64(dict, kCGWindowLayer).unwrap_or(0) != 0 {
                continue;
            }
            let Some(pid) =
                cf_dictionary_i64(dict, kCGWindowOwnerPID).and_then(|pid| u32::try_from(pid).ok())
            else {
                continue;
            };
            if pid != pid_filter {
                continue;
            }
            let Some(window_id) =
                cf_dictionary_i64(dict, kCGWindowNumber).and_then(|id| u32::try_from(id).ok())
            else {
                continue;
            };
            candidates.push(CgWindowCandidate {
                window_id,
                title: cf_dictionary_string(dict, kCGWindowName),
                bounds: cf_dictionary_bounds(dict, kCGWindowBounds),
            });
        }
        CFRelease(array as CFTypeRef);
    }
    Ok(candidates)
}

#[cfg(target_os = "macos")]
fn cg_window_id_for_ax_window(
    candidates: &[CgWindowCandidate],
    title: Option<&str>,
    position: Option<CGPoint>,
    size: Option<CGSize>,
) -> Option<i64> {
    if let (Some(position), Some(size)) = (position, size) {
        let mut matches = candidates.iter().filter(|candidate| {
            candidate
                .bounds
                .is_some_and(|bounds| bounds.matches(position, size))
        });
        let first = matches.next();
        if let Some(first) = first {
            if matches.next().is_none() {
                return Some(i64::from(first.window_id));
            }
        }
    }

    let title = title?;
    let mut matches = candidates
        .iter()
        .filter(|candidate| candidate.title.as_deref() == Some(title));
    let first = matches.next()?;
    if matches.next().is_none() {
        Some(i64::from(first.window_id))
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
impl CgWindowBounds {
    fn matches(self, position: CGPoint, size: CGSize) -> bool {
        (self.x - position.x).abs() <= 2.0
            && (self.y - position.y).abs() <= 2.0
            && (self.width - size.width).abs() <= 2.0
            && (self.height - size.height).abs() <= 2.0
    }
}

#[cfg(target_os = "macos")]
fn capture_cg_window(
    window_id: u32,
    max_width: u32,
    max_height: u32,
) -> Result<Option<WindowPreview>, HelperError> {
    let image = unsafe {
        CGWindowListCreateImage(
            cg_rect_null(),
            K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
            window_id,
            K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING,
        )
    };
    if image.is_null() {
        return Ok(None);
    }

    let result = unsafe {
        let width = CGImageGetWidth(image);
        let height = CGImageGetHeight(image);
        let bytes_per_row = CGImageGetBytesPerRow(image);
        let bits_per_pixel = CGImageGetBitsPerPixel(image);
        if width == 0 || height == 0 || bytes_per_row == 0 || bits_per_pixel != 32 {
            CFRelease(image as CFTypeRef);
            return Ok(None);
        }

        let provider = CGImageGetDataProvider(image);
        if provider.is_null() {
            CFRelease(image as CFTypeRef);
            return Ok(None);
        }
        let data = CGDataProviderCopyData(provider);
        if data.is_null() {
            CFRelease(image as CFTypeRef);
            return Ok(None);
        }

        let len = CFDataGetLength(data);
        let ptr = CFDataGetBytePtr(data);
        if len <= 0 || ptr.is_null() {
            CFRelease(data as CFTypeRef);
            CFRelease(image as CFTypeRef);
            return Ok(None);
        }
        let source = std::slice::from_raw_parts(ptr, len as usize);
        let preview = if source_has_visible_pixels(source) {
            downsample_bgra(
                source,
                width as u32,
                height as u32,
                bytes_per_row,
                max_width,
                max_height,
            )
        } else {
            None
        };
        CFRelease(data as CFTypeRef);
        CFRelease(image as CFTypeRef);
        Ok(preview)
    };
    result
}

#[cfg(target_os = "macos")]
fn source_has_visible_pixels(source: &[u8]) -> bool {
    let mut visible = 0_usize;
    let mut total = 0_usize;
    for pixel in source.chunks_exact(4).step_by(16) {
        total += 1;
        if pixel[0] > 8 || pixel[1] > 8 || pixel[2] > 8 {
            visible += 1;
        }
    }
    total > 0 && visible.saturating_mul(100) / total > 1
}

#[cfg(target_os = "macos")]
fn downsample_bgra(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    source_stride: usize,
    max_width: u32,
    max_height: u32,
) -> Option<WindowPreview> {
    let max_width = max_width.max(1);
    let max_height = max_height.max(1);
    let scale_w = max_width as f64 / source_width as f64;
    let scale_h = max_height as f64 / source_height as f64;
    let scale = scale_w.min(scale_h).min(1.0);
    let width = ((source_width as f64 * scale).round() as u32).max(1);
    let height = ((source_height as f64 * scale).round() as u32).max(1);
    let bytes_per_row = width as usize * 4;
    let mut bgra = vec![0_u8; bytes_per_row * height as usize];

    for y in 0..height {
        let src_y = (u64::from(y) * u64::from(source_height) / u64::from(height)) as usize;
        for x in 0..width {
            let src_x = (u64::from(x) * u64::from(source_width) / u64::from(width)) as usize;
            let src = src_y
                .saturating_mul(source_stride)
                .saturating_add(src_x.saturating_mul(4));
            let dst = y as usize * bytes_per_row + x as usize * 4;
            if src + 4 <= source.len() && dst + 4 <= bgra.len() {
                bgra[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
            }
        }
    }

    Some(WindowPreview {
        width,
        height,
        bytes_per_row,
        bgra,
    })
}

#[cfg(target_os = "macos")]
fn cf_dictionary_i64(dict: CFDictionaryRef, key: CFStringRef) -> Option<i64> {
    let mut value = std::ptr::null();
    let present = unsafe { CFDictionaryGetValueIfPresent(dict, key.cast(), &mut value) } != 0;
    if !present || value.is_null() {
        return None;
    }
    cf_number_to_i64(value)
}

#[cfg(target_os = "macos")]
fn cf_dictionary_string(dict: CFDictionaryRef, key: CFStringRef) -> Option<String> {
    let mut value = std::ptr::null();
    let present = unsafe { CFDictionaryGetValueIfPresent(dict, key.cast(), &mut value) } != 0;
    if !present || value.is_null() {
        return None;
    }
    cf_string_to_string(value as CFStringRef).and_then(|value| non_empty_string(&value))
}

#[cfg(target_os = "macos")]
fn cf_dictionary_bounds(dict: CFDictionaryRef, key: CFStringRef) -> Option<CgWindowBounds> {
    let mut value = std::ptr::null();
    let present = unsafe { CFDictionaryGetValueIfPresent(dict, key.cast(), &mut value) } != 0;
    if !present || value.is_null() {
        return None;
    }
    let bounds = value as CFDictionaryRef;
    Some(CgWindowBounds {
        x: cf_bounds_number(bounds, c"X")?,
        y: cf_bounds_number(bounds, c"Y")?,
        width: cf_bounds_number(bounds, c"Width")?,
        height: cf_bounds_number(bounds, c"Height")?,
    })
}

#[cfg(target_os = "macos")]
fn cf_bounds_number(bounds: CFDictionaryRef, key: &CStr) -> Option<f64> {
    let key = create_cf_string(key).ok()?;
    let mut value = std::ptr::null();
    let present = unsafe { CFDictionaryGetValueIfPresent(bounds, key.cast(), &mut value) } != 0;
    unsafe { CFRelease(key as CFTypeRef) };
    if !present || value.is_null() {
        return None;
    }
    cf_number_to_f64(value)
}

#[cfg(target_os = "macos")]
fn cf_number_to_i64(value: CFTypeRef) -> Option<i64> {
    let mut out = 0_i64;
    let ok = unsafe {
        CFNumberGetValue(
            value,
            K_CF_NUMBER_SINT64_TYPE,
            (&mut out as *mut i64).cast(),
        )
    } != 0;
    ok.then_some(out)
}

#[cfg(target_os = "macos")]
fn cf_number_to_f64(value: CFTypeRef) -> Option<f64> {
    let mut out = 0_f64;
    let ok = unsafe {
        CFNumberGetValue(
            value,
            K_CF_NUMBER_DOUBLE_TYPE,
            (&mut out as *mut f64).cast(),
        )
    } != 0;
    ok.then_some(out)
}

#[cfg(target_os = "macos")]
type Boolean = u8;
#[cfg(target_os = "macos")]
type AXError = i32;
#[cfg(target_os = "macos")]
type AXValueType = i32;
#[cfg(target_os = "macos")]
type AXUIElementRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type AXValueRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CFArrayRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CFDictionaryRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CFIndex = libc::c_long;
#[cfg(target_os = "macos")]
type CFStringRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CFTypeRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CGDataProviderRef = *const libc::c_void;
#[cfg(target_os = "macos")]
type CGImageRef = *const libc::c_void;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[cfg(target_os = "macos")]
const K_CG_WINDOW_LIST_OPTION_ALL: u32 = 0;
#[cfg(target_os = "macos")]
const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
#[cfg(target_os = "macos")]
const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
#[cfg(target_os = "macos")]
const K_CF_NUMBER_SINT64_TYPE: i32 = 4;
#[cfg(target_os = "macos")]
const K_CF_NUMBER_DOUBLE_TYPE: i32 = 13;
#[cfg(target_os = "macos")]
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
#[cfg(target_os = "macos")]
const K_AX_ERROR_SUCCESS: AXError = 0;
#[cfg(target_os = "macos")]
const K_AX_VALUE_CGPOINT_TYPE: AXValueType = 1;
#[cfg(target_os = "macos")]
const K_AX_VALUE_CGSIZE_TYPE: AXValueType = 2;

#[cfg(target_os = "macos")]
fn cg_rect_null() -> CGRect {
    CGRect {
        origin: CGPoint {
            x: f64::INFINITY,
            y: f64::INFINITY,
        },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    }
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kCGWindowLayer: CFStringRef;
    static kCGWindowBounds: CFStringRef;
    static kCGWindowName: CFStringRef;
    static kCGWindowNumber: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXValueGetValue(
        value: AXValueRef,
        the_type: AXValueType,
        value_ptr: *mut libc::c_void,
    ) -> Boolean;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGWindowListCreateImage(
        screen_bounds: CGRect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> CGImageRef;
    fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFTypeRef;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanFalse: CFTypeRef;
    fn CFArrayGetCount(the_array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(the_array: CFArrayRef, idx: CFIndex) -> *const libc::c_void;
    fn CFDictionaryGetValueIfPresent(
        the_dict: CFDictionaryRef,
        key: *const libc::c_void,
        value: *mut *const libc::c_void,
    ) -> Boolean;
    fn CFNumberGetValue(
        number: *const libc::c_void,
        the_type: i32,
        value_ptr: *mut libc::c_void,
    ) -> Boolean;
    fn CFDataGetBytePtr(the_data: CFTypeRef) -> *const u8;
    fn CFDataGetLength(the_data: CFTypeRef) -> CFIndex;
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithCString(
        allocator: *const libc::c_void,
        c_str: *const libc::c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetCString(
        the_string: CFStringRef,
        buffer: *mut libc::c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> Boolean;
}

#[cfg(target_os = "macos")]
fn set_visible_by_window_id(window: &WindowRef, visible: bool) -> Result<bool, HelperError> {
    let (Some(pid), Some(window_id)) = (window.pid, window.window_id) else {
        return Ok(false);
    };
    let windows_attr = create_cf_string(c"AXWindows")?;
    let minimized_attr = create_cf_string(c"AXMinimized")?;
    let window_number_attr = create_cf_string(c"AXWindowNumber")?;
    let raise_action = create_cf_string(c"AXRaise")?;
    let app = unsafe { AXUIElementCreateApplication(pid as libc::pid_t) };
    if app.is_null() {
        unsafe {
            CFRelease(windows_attr as CFTypeRef);
            CFRelease(minimized_attr as CFTypeRef);
            CFRelease(window_number_attr as CFTypeRef);
            CFRelease(raise_action as CFTypeRef);
        }
        return Err(HelperError::CoreGraphics(format!(
            "AXUIElementCreateApplication returned null for pid {pid}"
        )));
    }

    let result = unsafe {
        let mut windows_value = std::ptr::null();
        let copy_error = AXUIElementCopyAttributeValue(app, windows_attr, &mut windows_value);
        if copy_error != K_AX_ERROR_SUCCESS || windows_value.is_null() {
            CFRelease(windows_attr as CFTypeRef);
            CFRelease(minimized_attr as CFTypeRef);
            CFRelease(window_number_attr as CFTypeRef);
            CFRelease(raise_action as CFTypeRef);
            CFRelease(app as CFTypeRef);
            return Err(HelperError::CoreGraphics(format!(
                "AX windows unavailable for pid {pid}: {copy_error}"
            )));
        }

        let windows = windows_value as CFArrayRef;
        let count = CFArrayGetCount(windows);
        let mut matched = false;
        let mut raise_result = Ok(());
        for index in 0..count {
            let ax_window = CFArrayGetValueAtIndex(windows, index) as AXUIElementRef;
            if ax_window.is_null() {
                continue;
            }
            let candidate_window_id = ax_cg_window_id(ax_window)
                .or_else(|| ax_attribute_i64(ax_window, window_number_attr));
            if candidate_window_id == Some(window_id) {
                matched = true;
                raise_result = raise_ax_window(ax_window, window, minimized_attr, raise_action);
                break;
            }
        }

        CFRelease(windows_attr as CFTypeRef);
        CFRelease(minimized_attr as CFTypeRef);
        CFRelease(window_number_attr as CFTypeRef);
        CFRelease(raise_action as CFTypeRef);
        CFRelease(windows_value);
        CFRelease(app as CFTypeRef);

        if !matched {
            if visible && restore_minimized_window_system_events(window)? {
                return Ok(true);
            }
            return Ok(false);
        }
        raise_result.map(|()| true)
    };
    result
}

#[cfg(target_os = "macos")]
fn set_visible_by_window_index(window: &WindowRef, visible: bool) -> Result<bool, HelperError> {
    let (Some(pid), Some(window_index)) = (window.pid, window.window_index) else {
        return Ok(false);
    };
    let windows_attr = create_cf_string(c"AXWindows")?;
    let minimized_attr = create_cf_string(c"AXMinimized")?;
    let title_attr = create_cf_string(c"AXTitle")?;
    let raise_action = create_cf_string(c"AXRaise")?;
    let app = unsafe { AXUIElementCreateApplication(pid as libc::pid_t) };
    if app.is_null() {
        unsafe {
            CFRelease(windows_attr as CFTypeRef);
            CFRelease(minimized_attr as CFTypeRef);
            CFRelease(title_attr as CFTypeRef);
            CFRelease(raise_action as CFTypeRef);
        }
        return Err(HelperError::CoreGraphics(format!(
            "AXUIElementCreateApplication returned null for pid {pid}"
        )));
    }

    let result = unsafe {
        let mut windows_value = std::ptr::null();
        let copy_error = AXUIElementCopyAttributeValue(app, windows_attr, &mut windows_value);
        if copy_error != K_AX_ERROR_SUCCESS || windows_value.is_null() {
            CFRelease(windows_attr as CFTypeRef);
            CFRelease(minimized_attr as CFTypeRef);
            CFRelease(title_attr as CFTypeRef);
            CFRelease(raise_action as CFTypeRef);
            CFRelease(app as CFTypeRef);
            return Err(HelperError::CoreGraphics(format!(
                "AX windows unavailable for pid {pid}: {copy_error}"
            )));
        }

        let windows = windows_value as CFArrayRef;
        let count = CFArrayGetCount(windows);
        let index = window_index.saturating_sub(1) as CFIndex;
        if index < 0 || index >= count {
            CFRelease(windows_attr as CFTypeRef);
            CFRelease(minimized_attr as CFTypeRef);
            CFRelease(title_attr as CFTypeRef);
            CFRelease(raise_action as CFTypeRef);
            CFRelease(windows_value);
            CFRelease(app as CFTypeRef);
            if visible && restore_minimized_window_system_events(window)? {
                return Ok(true);
            }
            return Ok(false);
        }

        let ax_window = CFArrayGetValueAtIndex(windows, index) as AXUIElementRef;
        let title_matches = match window.title.as_deref() {
            Some(expected) => {
                ax_attribute_string(ax_window, title_attr).as_deref() == Some(expected)
            }
            None => true,
        };
        if !title_matches {
            CFRelease(windows_attr as CFTypeRef);
            CFRelease(minimized_attr as CFTypeRef);
            CFRelease(title_attr as CFTypeRef);
            CFRelease(raise_action as CFTypeRef);
            CFRelease(windows_value);
            CFRelease(app as CFTypeRef);
            if visible && restore_minimized_window_system_events(window)? {
                return Ok(true);
            }
            return Ok(false);
        }

        let raise_result = raise_ax_window(ax_window, window, minimized_attr, raise_action);

        CFRelease(windows_attr as CFTypeRef);
        CFRelease(minimized_attr as CFTypeRef);
        CFRelease(title_attr as CFTypeRef);
        CFRelease(raise_action as CFTypeRef);
        CFRelease(windows_value);
        CFRelease(app as CFTypeRef);

        raise_result.map(|()| true)
    };
    result
}

#[cfg(target_os = "macos")]
fn raise_ax_window(
    ax_window: AXUIElementRef,
    window: &WindowRef,
    minimized_attr: CFStringRef,
    raise_action: CFStringRef,
) -> Result<(), HelperError> {
    let _ = unsafe { AXUIElementSetAttributeValue(ax_window, minimized_attr, kCFBooleanFalse) };
    let raise_error = unsafe { AXUIElementPerformAction(ax_window, raise_action) };
    if raise_error != K_AX_ERROR_SUCCESS {
        return Err(HelperError::CoreGraphics(format!(
            "AXRaise failed for pid {:?} window id {:?} index {:?}: {raise_error}",
            window.pid, window.window_id, window.window_index
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn restore_minimized_window_system_events(window: &WindowRef) -> Result<bool, HelperError> {
    let (Some(pid), Some(title)) = (window.pid, window.title.as_deref()) else {
        return Ok(false);
    };
    let title = apple_script_string(title);
    let script = format!(
        r#"tell application "System Events"
    set pidFilter to {pid}
    set titleFilter to {title}
    repeat with p in application processes
        set processPid to 0
        try
            set processPid to unix id of p
        end try
        if processPid is pidFilter then
            repeat with w in windows of p
                set windowTitle to ""
                try
                    set windowTitle to name of w
                end try
                if windowTitle is titleFilter then
                    try
                        set value of attribute "AXMinimized" of w to false
                    end try
                    try
                        perform action "AXRaise" of w
                    end try
                    return "1"
                end if
            end repeat
        end if
    end repeat
    return "0"
end tell"#
    );
    run_osascript(&script).map(|out| out.trim() == "1")
}

#[cfg(target_os = "macos")]
fn create_cf_string(value: &CStr) -> Result<CFStringRef, HelperError> {
    let string = unsafe {
        CFStringCreateWithCString(std::ptr::null(), value.as_ptr(), K_CF_STRING_ENCODING_UTF8)
    };
    if string.is_null() {
        Err(HelperError::CoreGraphics(format!(
            "failed to create CFString for {}",
            value.to_string_lossy()
        )))
    } else {
        Ok(string)
    }
}

#[cfg(target_os = "macos")]
fn ax_attribute_i64(element: AXUIElementRef, attribute: CFStringRef) -> Option<i64> {
    let mut value = std::ptr::null();
    let error = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    if error != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    let number = cf_number_to_i64(value);
    unsafe { CFRelease(value) };
    number
}

#[cfg(target_os = "macos")]
fn ax_attribute_string(element: AXUIElementRef, attribute: CFStringRef) -> Option<String> {
    let mut value = std::ptr::null();
    let error = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    if error != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    let string = cf_string_to_string(value as CFStringRef);
    unsafe { CFRelease(value) };
    string.and_then(|value| non_empty_string(&value))
}

#[cfg(target_os = "macos")]
fn ax_cg_window_id(element: AXUIElementRef) -> Option<i64> {
    type AxElementGetWindow =
        unsafe extern "C" fn(element: AXUIElementRef, out: *mut u32) -> AXError;

    let symbol =
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"_AXUIElementGetWindow".as_ptr().cast()) };
    if symbol.is_null() {
        return None;
    }
    let get_window: AxElementGetWindow = unsafe { std::mem::transmute(symbol) };
    let mut out = 0_u32;
    let error = unsafe { get_window(element, &mut out) };
    (error == K_AX_ERROR_SUCCESS && out > 0).then_some(i64::from(out))
}

#[cfg(target_os = "macos")]
fn ax_attribute_point(element: AXUIElementRef, attribute: CFStringRef) -> Option<CGPoint> {
    let mut value = std::ptr::null();
    let error = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    if error != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe {
        AXValueGetValue(
            value as AXValueRef,
            K_AX_VALUE_CGPOINT_TYPE,
            (&mut point as *mut CGPoint).cast(),
        )
    } != 0;
    unsafe { CFRelease(value) };
    ok.then_some(point)
}

#[cfg(target_os = "macos")]
fn ax_attribute_size(element: AXUIElementRef, attribute: CFStringRef) -> Option<CGSize> {
    let mut value = std::ptr::null();
    let error = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    if error != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok = unsafe {
        AXValueGetValue(
            value as AXValueRef,
            K_AX_VALUE_CGSIZE_TYPE,
            (&mut size as *mut CGSize).cast(),
        )
    } != 0;
    unsafe { CFRelease(value) };
    ok.then_some(size)
}

#[cfg(target_os = "macos")]
fn cf_string_to_string(value: CFStringRef) -> Option<String> {
    let mut buffer = [0_i8; 4096];
    let ok = unsafe {
        CFStringGetCString(
            value,
            buffer.as_mut_ptr(),
            buffer.len() as CFIndex,
            K_CF_STRING_ENCODING_UTF8,
        )
    } != 0;
    if !ok {
        return None;
    }
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

pub fn set_visible(window: &WindowRef, visible: bool) -> Result<(), HelperError> {
    let started = Instant::now();
    let result = set_visible_inner(window, visible);
    tracing::debug!(
        operation = "macos.helper.set_visible",
        duration_ms = elapsed_ms(started),
        visible,
        pid = ?window.pid,
        bundle_id = ?window.bundle_id,
        window_id = ?window.window_id,
        window_index = ?window.window_index,
        ok = result.is_ok(),
        error = result.as_ref().err().map(ToString::to_string),
        "macOS helper set visible finished"
    );
    result
}

fn set_visible_inner(window: &WindowRef, visible: bool) -> Result<(), HelperError> {
    if window.pid.is_none() && window.bundle_id.is_none() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if !visible {
        // Anchor switching on macOS is intentionally non-destructive:
        // inactive anchor windows are left alone, while activating an anchor
        // only raises the windows explicitly attached to it.
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if window.window_id.is_some() {
        match set_visible_by_window_id_for_inner(window, visible) {
            Ok(true) if !visible => return Ok(()),
            Ok(true) => return Ok(()),
            Ok(false) => {
                return Err(HelperError::CoreGraphics(format!(
                    "window id {} not found for pid {:?}",
                    window.window_id.unwrap_or_default(),
                    window.pid
                )));
            }
            Err(err) => return Err(err),
        }
    }

    #[cfg(target_os = "macos")]
    if window.window_index.is_some() {
        match set_visible_by_window_index(window, visible) {
            Ok(true) => return Ok(()),
            Ok(false) => {
                return Err(HelperError::CoreGraphics(format!(
                    "window index {:?} not found for pid {:?} title {:?}",
                    window.window_index, window.pid, window.title
                )));
            }
            Err(err) => return Err(err),
        }
    }

    #[cfg(target_os = "macos")]
    {
        return Err(HelperError::CoreGraphics(format!(
            "refusing macOS visibility change without exact window id for pid {:?} bundle {:?}",
            window.pid, window.bundle_id
        )));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, visible);
        Ok(())
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn set_visible_by_window_id_for_inner(
    window: &WindowRef,
    visible: bool,
) -> Result<bool, HelperError> {
    set_visible_by_window_id(window, visible)
}

#[cfg(all(target_os = "macos", test))]
fn set_visible_by_window_id_for_inner(
    window: &WindowRef,
    visible: bool,
) -> Result<bool, HelperError> {
    if let Some(result) = TEST_WINDOW_ID_VISIBILITY_RESULT
        .lock()
        .ok()
        .and_then(|guard| *guard)
    {
        return Ok(result);
    }
    set_visible_by_window_id(window, visible)
}

fn run_osascript(script: &str) -> Result<String, HelperError> {
    #[cfg(test)]
    TEST_OSASCRIPT_CALLS.fetch_add(1, Ordering::SeqCst);
    let output = Command::new("osascript").args(["-e", script]).output()?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    Err(HelperError::Osascript(
        String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    ))
}

fn apple_script_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.is_empty() || value == "missing value" {
        None
    } else {
        Some(value.to_owned())
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    let millis = started.elapsed().as_millis();
    millis.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn request_serializes_as_tagged_protocol() {
        let request = HelperRequest::SetVisible {
            window: WindowRef {
                pid: Some(42),
                bundle_id: Some("com.example.App".into()),
                window_index: Some(1),
                window_id: None,
                title: None,
            },
            visible: false,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""cmd":"set_visible""#));
        assert!(json.contains(r#""pid":42"#));
        assert!(json.contains(r#""visible":false"#));
    }

    #[test]
    fn invalid_stdin_line_returns_error_response() {
        let mut out = Vec::new();

        run_stdio("not-json\n".as_bytes(), &mut out).unwrap();

        let line = String::from_utf8(out).unwrap();
        let response: HelperResponse = serde_json::from_str(line.trim()).unwrap();
        assert!(matches!(response, HelperResponse::Error { .. }));
    }

    #[test]
    fn apple_script_string_escapes_quotes_and_backslashes() {
        assert_eq!(
            apple_script_string(r#"com.example."quoted"\app"#),
            r#""com.example.\"quoted\"\\app""#
        );
    }

    #[test]
    fn windows_response_preserves_stable_window_ids() {
        let response = HelperResponse::Windows {
            windows: vec![
                WindowInfo {
                    window_id: Some(1001),
                    pid: 42,
                    bundle_id: Some("com.example.App".into()),
                    window_index: 1,
                    title: Some("First".into()),
                },
                WindowInfo {
                    window_id: Some(1002),
                    pid: 42,
                    bundle_id: Some("com.example.App".into()),
                    window_index: 2,
                    title: Some("Second".into()),
                },
            ],
        };

        let json = serde_json::to_string(&response).unwrap();
        let decoded: HelperResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, response);
    }

    #[test]
    fn focused_window_response_roundtrips_absent_window() {
        let response = HelperResponse::FocusedWindow { window: None };

        let json = serde_json::to_string(&response).unwrap();
        let decoded: HelperResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, response);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_application_processes_and_deduplicates_pids() {
        let processes = parse_application_processes(
            "42\tcom.example.First\n42\tcom.example.First\n77\tmissing value\nbad\trow\n",
        )
        .unwrap();

        assert_eq!(
            processes,
            vec![
                AppProcess {
                    pid: 42,
                    bundle_id: Some("com.example.First".into()),
                },
                AppProcess {
                    pid: 77,
                    bundle_id: None,
                },
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_window_id_success_does_not_invoke_osascript() {
        if let Ok(mut result) = TEST_WINDOW_ID_VISIBILITY_RESULT.lock() {
            *result = Some(true);
        }
        TEST_OSASCRIPT_CALLS.store(0, Ordering::SeqCst);

        let result = set_visible(
            &WindowRef {
                pid: Some(42),
                bundle_id: Some("com.example.App".into()),
                window_index: None,
                window_id: Some(1001),
                title: None,
            },
            true,
        );

        if let Ok(mut result) = TEST_WINDOW_ID_VISIBILITY_RESULT.lock() {
            *result = None;
        }
        assert!(result.is_ok());
        assert_eq!(TEST_OSASCRIPT_CALLS.load(Ordering::SeqCst), 0);
    }
}
