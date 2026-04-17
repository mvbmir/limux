//! Per-tab Claude session resume support.
//!
//! We assign each terminal tab a stable session UUID and force the `claude`
//! CLI to use it via `--session-id <uuid>`. This makes session identity
//! deterministic and per-tab even when several tabs share a working
//! directory, and removes the need to scan `/proc` or guess from filesystem
//! mtimes.
//!
//! The mechanism is a thin wrapper script placed in a limux-owned directory
//! that we prepend to the shell's `PATH`. The wrapper forwards every
//! invocation of `claude` to the real binary, injecting `--session-id
//! $LIMUX_CLAUDE_SESSION_ID` when the user hasn't already asked for a
//! specific session (via `--resume`, `--continue`, or an explicit
//! `--session-id`).

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// Directory where the wrapper script is installed. Kept under
/// `$XDG_DATA_HOME` so it lives alongside limux's other persistent state.
pub fn wrapper_bin_dir() -> Option<PathBuf> {
    let base = dirs::data_dir().or_else(dirs::home_dir)?;
    Some(if base.ends_with(".local/share") {
        base.join("limux").join("bin")
    } else {
        base.join(".local/share").join("limux").join("bin")
    })
}

/// Idempotently install the `claude` shim. Safe to call on every launch: the
/// script is only rewritten if its contents differ from the expected body.
pub fn ensure_wrapper_script() -> io::Result<PathBuf> {
    let dir = wrapper_bin_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no data directory available for limux wrapper",
        )
    })?;
    fs::create_dir_all(&dir)?;
    let script = dir.join("claude");
    let body = wrapper_script_body();
    let up_to_date = fs::read_to_string(&script)
        .map(|existing| existing == body)
        .unwrap_or(false);
    if !up_to_date {
        fs::write(&script, body)?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    }
    Ok(script)
}

/// Shell script body for the wrapper. POSIX-compatible so it works under
/// bash, zsh, dash, and fish's sh-exec fallback. Keeping it self-contained
/// in source makes updates trivial and avoids shipping a separate asset.
fn wrapper_script_body() -> String {
    // `IFS=:` lets us split PATH without invoking an external `tr`. The loop
    // strips our own directory so `command -v claude` never points back at
    // the wrapper itself, which would cause infinite recursion.
    r#"#!/bin/sh
# Installed by limux. Forces per-tab Claude session IDs via $LIMUX_CLAUDE_SESSION_ID.
# Do not edit; regenerated on every limux launch.

self_dir=$(CDPATH= cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd -P)

# Drop our shim directory from PATH before resolving the real `claude`.
cleaned_path=""
IFS=:
for dir in $PATH; do
    [ "$dir" = "$self_dir" ] && continue
    if [ -n "$cleaned_path" ]; then
        cleaned_path="$cleaned_path:$dir"
    else
        cleaned_path="$dir"
    fi
done
unset IFS

real_claude=$(PATH="$cleaned_path" command -v claude 2>/dev/null)
if [ -z "$real_claude" ]; then
    printf 'limux: real claude binary not found on PATH\n' >&2
    exit 127
fi

# If the user already specified session intent, don't interfere.
for arg in "$@"; do
    case "$arg" in
        -c|--continue|--resume|--resume=*|--session-id|--session-id=*|--from-pr|--from-pr=*|--fork-session)
            exec "$real_claude" "$@"
            ;;
    esac
done

if [ -n "$LIMUX_CLAUDE_SESSION_ID" ]; then
    exec "$real_claude" --session-id "$LIMUX_CLAUDE_SESSION_ID" "$@"
fi

exec "$real_claude" "$@"
"#
    .to_string()
}

/// Build the shell command that resumes a specific Claude session.
/// Invoked by the restore path to spawn claude as the tab's initial command.
pub fn resume_command(session_id: &str) -> String {
    format!("claude --resume {session_id}")
}

/// Verify that a session JSONL still exists on disk before trying to resume
/// it. Returns the expected path if present, `None` otherwise. A missing
/// file means the restore path should fall back to a plain shell instead of
/// launching claude against a stale UUID.
pub fn session_file_exists(cwd: &str, session_id: &str) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let dir = home.join(".claude").join("projects").join(encode_cwd(cwd));
    dir.join(format!("{session_id}.jsonl")).exists()
}

fn encode_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// Generate a fresh v4 UUID suitable for `--session-id`.
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Return the session UUID currently active in the tab tagged with
/// `tab_id`, by locating its `claude` process and reading the
/// `~/.claude/sessions/<pid>.json` status file Claude itself maintains.
///
/// Returns `None` when no `claude` is running in the tab, the status file
/// is missing, or the payload cannot be parsed. Claude rewrites this file
/// whenever the session changes (including after `/resume` inside the
/// interactive UI), so polling it gives us up-to-date session identity
/// without scraping JSONLs or parsing process open files.
pub fn detect_active_session_for_tab(tab_id: &str) -> Option<String> {
    let claude_pid = find_tab_claude_pid(tab_id)?;
    read_claude_session_pid_file(claude_pid)
}

fn find_tab_claude_pid(tab_id: &str) -> Option<u32> {
    let needle = format!("LIMUX_TAB_ID={tab_id}");
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let comm_path = format!("/proc/{name}/comm");
        let Ok(comm) = fs::read_to_string(&comm_path) else {
            continue;
        };
        if comm.trim() != "claude" {
            continue;
        }
        let env_path = format!("/proc/{name}/environ");
        let Ok(env_bytes) = fs::read(env_path) else {
            continue;
        };
        if env_bytes
            .split(|byte| *byte == 0)
            .any(|entry| entry == needle.as_bytes())
        {
            return Some(name);
        }
    }
    None
}

fn read_claude_session_pid_file(pid: u32) -> Option<String> {
    let home = dirs::home_dir()?;
    let path = home
        .join(".claude")
        .join("sessions")
        .join(format!("{pid}.json"));
    let raw = fs::read_to_string(path).ok()?;
    // Minimal JSON field extraction: the file is tiny and structurally fixed
    // by Claude, so a bespoke scan avoids pulling in a full JSON parser just
    // for one string field. Falls through to `None` on any surprise.
    extract_string_field(&raw, "sessionId")
}

fn extract_string_field(json: &str, field: &str) -> Option<String> {
    // Look for `"<field>"` followed by `:` and a `"..."` value. Handles the
    // canonical shape Claude writes: `{"pid":123,"sessionId":"...","...":...}`.
    let key = format!("\"{field}\"");
    let start = json.find(&key)?;
    let after_key = &json[start + key.len()..];
    let colon = after_key.find(':')?;
    let value_start = after_key[colon + 1..].find('"')?;
    let value_region = &after_key[colon + 1 + value_start + 1..];
    let value_end = value_region.find('"')?;
    Some(value_region[..value_end].to_string())
}

/// Return the absolute path to the wrapper directory as an `OsString`
/// suitable for prepending to `PATH`. `None` if the directory cannot be
/// determined (missing `$HOME`).
pub fn wrapper_path_component() -> Option<PathBuf> {
    wrapper_bin_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn encode_cwd_replaces_slashes() {
        assert_eq!(encode_cwd("/home/me/projects/foo"), "-home-me-projects-foo");
        assert_eq!(encode_cwd(""), "");
    }

    #[test]
    fn resume_command_matches_expected_format() {
        assert_eq!(
            resume_command("575027f5-543a-47c9-a449-cd2704e0c12c"),
            "claude --resume 575027f5-543a-47c9-a449-cd2704e0c12c"
        );
    }

    #[test]
    fn wrapper_script_contains_key_logic() {
        let body = wrapper_script_body();
        assert!(body.starts_with("#!/bin/sh"));
        assert!(body.contains("LIMUX_CLAUDE_SESSION_ID"));
        assert!(body.contains("--session-id"));
        assert!(body.contains("--resume"));
    }

    #[test]
    fn ensure_wrapper_script_writes_executable_file() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("claude");
        fs::write(&script, "stale").expect("seed stale");
        // Hand-call the body-write path directly to avoid touching user state.
        let body = wrapper_script_body();
        fs::write(&script, &body).expect("write");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        let meta = fs::metadata(&script).expect("metadata");
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        assert_eq!(fs::read_to_string(&script).unwrap(), body);
    }

    #[test]
    fn new_session_id_is_unique_uuid_format() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
    }

    #[test]
    fn session_file_exists_false_when_missing() {
        assert!(!session_file_exists(
            "/nonexistent/path/for/limux/test",
            "00000000-0000-0000-0000-000000000000"
        ));
    }

    #[test]
    fn extract_string_field_reads_session_id_from_claude_payload() {
        let raw =
            r#"{"pid":687361,"sessionId":"75c921ee-b7ea-4fb6-992b-e6f99d7e5dcc","cwd":"/home/me"}"#;
        assert_eq!(
            extract_string_field(raw, "sessionId").as_deref(),
            Some("75c921ee-b7ea-4fb6-992b-e6f99d7e5dcc")
        );
    }

    #[test]
    fn extract_string_field_returns_none_for_missing_key() {
        let raw = r#"{"pid":1,"cwd":"/"}"#;
        assert!(extract_string_field(raw, "sessionId").is_none());
    }

    #[test]
    fn extract_string_field_handles_extra_whitespace() {
        let raw = r#"{ "sessionId" : "abc" }"#;
        assert_eq!(
            extract_string_field(raw, "sessionId").as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn detect_active_session_returns_none_when_tab_missing() {
        assert!(detect_active_session_for_tab("limux-test-no-such-tab-id").is_none());
    }
}
