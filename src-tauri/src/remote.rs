//! Typed wrapper around the bundled rclone sidecar. ClipSnip owns a private
//! rclone config holding exactly one remote named `clipsnip`; every command
//! pins `--config` so the user's own rclone setup is never touched. Arg
//! builders are pure so they can be unit tested; the async runner lives in
//! lib.rs where the AppHandle is available.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const REMOTE_NAME: &str = "clipsnip";

#[derive(Debug, Clone)]
pub enum SftpAuth {
    Password(String),
    KeyFile(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    pub path: String,
    pub size: u64,
    pub mod_time: i64,
}

pub fn config_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("rclone.conf")
}

pub fn base_args(config_path: &Path) -> Vec<String> {
    vec![
        "--config".into(),
        config_path.to_string_lossy().to_string(),
        "--auto-confirm".into(),
        "--log-level".into(),
        "ERROR".into(),
    ]
}

pub fn create_sftp_args(
    config_path: &Path,
    host: &str,
    port: u16,
    user: &str,
    auth: &SftpAuth,
) -> Vec<String> {
    let mut args = base_args(config_path);
    if matches!(auth, SftpAuth::Password(_)) {
        args.push("--obscure".into());
    }
    args.extend([
        "config".into(), "create".into(), REMOTE_NAME.into(), "sftp".into(),
        format!("host={host}"), format!("port={port}"), format!("user={user}"),
    ]);
    match auth {
        SftpAuth::Password(p) => args.push(format!("pass={p}")),
        SftpAuth::KeyFile(k) => args.push(format!("key_file={k}")),
    }
    args
}

/// `provider` is an rclone backend name: "dropbox" or "drive". Running this
/// starts rclone's local OAuth flow: it opens the system browser and blocks
/// until the user approves (or fails). No client registration needed.
pub fn create_oauth_args(config_path: &Path, provider: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend([
        "config".into(), "create".into(), REMOTE_NAME.into(), provider.into(),
    ]);
    args
}

pub fn delete_remote_args(config_path: &Path) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["config".into(), "delete".into(), REMOTE_NAME.into()]);
    args
}

/// Builds `clipsnip:<base>/ClipSnip[/<sub>]`. Empty sub returns the ClipSnip
/// root itself. Trailing slashes on base are tolerated.
pub fn remote_dir(base_path: Option<&str>, sub: &str) -> String {
    let base = base_path.unwrap_or("").trim_matches('/');
    let mut out = format!("{REMOTE_NAME}:");
    if !base.is_empty() {
        out.push_str(base);
        out.push('/');
    }
    out.push_str("ClipSnip");
    if !sub.is_empty() {
        out.push('/');
        out.push_str(sub);
    }
    out
}

pub fn mkdir_args(config_path: &Path, dir: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["mkdir".into(), dir.into()]);
    args
}

pub fn lsd_check_args(config_path: &Path, dir: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["lsd".into(), dir.into()]);
    args
}

pub fn lsjson_args(config_path: &Path, dir: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["lsjson".into(), "-R".into(), "--files-only".into(), dir.into()]);
    args
}

pub fn copyto_args(config_path: &Path, local: &str, remote: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["copyto".into(), local.into(), remote.into()]);
    args
}

pub fn download_args(config_path: &Path, remote: &str, local: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["copyto".into(), remote.into(), local.into()]);
    args
}

pub fn deletefile_args(config_path: &Path, remote: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["deletefile".into(), remote.into()]);
    args
}

#[derive(Debug, Deserialize)]
struct LsJsonEntry {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Size")]
    size: i64,
    #[serde(rename = "ModTime")]
    mod_time: String,
    #[serde(rename = "IsDir", default)]
    is_dir: bool,
}

/// Parses `rclone lsjson -R --files-only` output. ModTime is RFC3339; parse
/// the "YYYY-MM-DDTHH:MM:SS" prefix manually (UTC assumed, fractional seconds
/// and offsets after the seconds field are ignored) to avoid a chrono dep.
pub fn parse_lsjson(json: &str) -> Result<Vec<RemoteFile>, String> {
    let entries: Vec<LsJsonEntry> =
        serde_json::from_str(json).map_err(|e| format!("bad lsjson output: {e}"))?;
    Ok(entries
        .into_iter()
        .filter(|e| !e.is_dir)
        .map(|e| RemoteFile {
            path: e.path,
            size: e.size.max(0) as u64,
            mod_time: rfc3339_to_unix(&e.mod_time),
        })
        .collect())
}

/// Minimal RFC3339 → unix seconds (UTC). Parses the date/time prefix
/// (YYYY-MM-DDTHH:MM:SS), skips optional fractional seconds (.digits),
/// then parses timezone: Z or end-of-string → UTC; +/-HH:MM, +/-HHMM, or +/-HH
/// → apply offset. Formula: unix = wall_clock_seconds − offset_seconds.
/// Returns 0 on unparseable date/time prefix; lenient about invalid suffix
/// (treats it as UTC). Mod-time is display-only (diff window date column),
/// never a sync decision.
fn rfc3339_to_unix(s: &str) -> i64 {
    fn num(s: &str, a: usize, b: usize) -> Option<i64> { s.get(a..b)?.parse().ok() }
    let (y, mo, d) = match (num(s, 0, 4), num(s, 5, 7), num(s, 8, 10)) {
        (Some(y), Some(mo), Some(d)) => (y, mo, d),
        _ => return 0,
    };
    let (h, mi, sec) = match (num(s, 11, 13), num(s, 14, 16), num(s, 17, 19)) {
        (Some(h), Some(mi), Some(sec)) => (h, mi, sec),
        _ => return 0,
    };
    // days since epoch via civil-days algorithm (Howard Hinnant's days_from_civil)
    let y_adj = if mo <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let wall_clock_seconds = days * 86400 + h * 3600 + mi * 60 + sec;

    // Parse timezone offset after the seconds field (position 19)
    let offset_seconds = if let Some(suffix) = s.get(19..) {
        // Skip optional fractional seconds: "." followed by digits
        let mut pos = 0;
        if suffix.starts_with('.') {
            pos = 1;
            while pos < suffix.len() && suffix.as_bytes()[pos].is_ascii_digit() {
                pos += 1;
            }
        }
        let tz_part = &suffix[pos..];

        if tz_part.is_empty() {
            // No timezone specified; assume UTC
            0
        } else if tz_part.starts_with('Z') {
            // UTC
            0
        } else if tz_part.starts_with('+') || tz_part.starts_with('-') {
            let sign = if tz_part.starts_with('+') { 1 } else { -1 };
            let tz = &tz_part[1..];

            // Try parsing HH:MM, HHMM, or HH
            let (hours, mins) = if let Some(colon_pos) = tz.find(':') {
                // Format: HH:MM
                match (tz.get(..colon_pos).and_then(|s| s.parse::<i64>().ok()),
                       tz.get(colon_pos + 1..).and_then(|s| s.parse::<i64>().ok())) {
                    (Some(h), Some(m)) => (h, m),
                    _ => (0, 0), // Invalid; treat as UTC
                }
            } else if tz.len() >= 4 {
                // Format: HHMM
                match (tz.get(..2).and_then(|s| s.parse::<i64>().ok()),
                       tz.get(2..4).and_then(|s| s.parse::<i64>().ok())) {
                    (Some(h), Some(m)) => (h, m),
                    _ => (0, 0),
                }
            } else if tz.len() >= 2 {
                // Format: HH
                match tz.get(..2).and_then(|s| s.parse::<i64>().ok()) {
                    Some(h) => (h, 0),
                    _ => (0, 0),
                }
            } else {
                (0, 0) // Invalid; treat as UTC
            };

            sign * (hours * 3600 + mins * 60)
        } else {
            // Invalid suffix; treat as UTC
            0
        }
    } else {
        // No suffix at all; assume UTC
        0
    };

    wall_clock_seconds - offset_seconds
}

pub fn identity(
    provider: &str,
    host: Option<&str>,
    user: Option<&str>,
    base_path: Option<&str>,
) -> String {
    let base = base_path.unwrap_or("");
    match provider {
        "sftp" => format!("sftp:{}@{}/{base}", user.unwrap_or(""), host.unwrap_or("")),
        p => format!("{p}/{base}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn cfg() -> &'static Path { Path::new("/cfg/rclone.conf") }

    #[test]
    fn base_args_pin_config_and_noninteractive_flags() {
        let args = base_args(cfg());
        assert_eq!(args, vec!["--config", "/cfg/rclone.conf", "--auto-confirm", "--log-level", "ERROR"]);
    }

    #[test]
    fn create_sftp_args_with_password_obscures_it() {
        let args = create_sftp_args(cfg(), "nas.local", 22, "milosz",
            &SftpAuth::Password("s3cret".into()));
        let joined = args.join(" ");
        assert!(joined.contains("config create clipsnip sftp"));
        assert!(joined.contains("host=nas.local"));
        assert!(joined.contains("port=22"));
        assert!(joined.contains("user=milosz"));
        assert!(joined.contains("pass=s3cret"));
        assert!(joined.contains("--obscure"));
    }

    #[test]
    fn create_sftp_args_with_key_file_has_no_pass() {
        let args = create_sftp_args(cfg(), "nas.local", 2222, "milosz",
            &SftpAuth::KeyFile("/home/m/.ssh/id_ed25519".into()));
        let joined = args.join(" ");
        assert!(joined.contains("key_file=/home/m/.ssh/id_ed25519"));
        assert!(!joined.contains("pass="));
        assert!(!joined.contains("--obscure"));
    }

    #[test]
    fn create_oauth_args_names_the_provider() {
        let joined = create_oauth_args(cfg(), "dropbox").join(" ");
        assert!(joined.contains("config create clipsnip dropbox"));
    }

    #[test]
    fn remote_dir_joins_base_path_and_defaults_to_root() {
        assert_eq!(remote_dir(Some("backups"), "Clips"), "clipsnip:backups/ClipSnip/Clips");
        assert_eq!(remote_dir(None, "Clips"), "clipsnip:ClipSnip/Clips");
        assert_eq!(remote_dir(Some("a/"), ""), "clipsnip:a/ClipSnip");
    }

    #[test]
    fn copyto_lsjson_deletefile_args_shape() {
        assert_eq!(copyto_args(cfg(), "/tmp/a.mp4", "clipsnip:ClipSnip/Clips/a.mp4")[..3].to_vec(),
            vec!["--config", "/cfg/rclone.conf", "--auto-confirm"]);
        let ls = lsjson_args(cfg(), "clipsnip:ClipSnip/Clips");
        assert!(ls.contains(&"lsjson".to_string()) && ls.contains(&"-R".to_string())
            && ls.contains(&"--files-only".to_string()));
        let del = deletefile_args(cfg(), "clipsnip:ClipSnip/Clips/a.mp4");
        assert!(del.contains(&"deletefile".to_string()));
    }

    #[test]
    fn parse_lsjson_maps_path_size_and_rfc3339_modtime() {
        let json = r#"[{"Path":"sub/a.mp4","Name":"a.mp4","Size":1048576,
            "ModTime":"2026-07-06T10:00:00Z","IsDir":false}]"#;
        let files = parse_lsjson(json).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "sub/a.mp4");
        assert_eq!(files[0].size, 1048576);
        assert_eq!(files[0].mod_time, 1783332000);
    }

    #[test]
    fn rfc3339_to_unix_handles_timezone_offsets_and_fractional_seconds() {
        // All represent 2026-07-06T10:00:00Z = 1783332000
        assert_eq!(rfc3339_to_unix("2026-07-06T10:00:00Z"), 1783332000);
        assert_eq!(rfc3339_to_unix("2026-07-06T12:00:00+02:00"), 1783332000);
        assert_eq!(rfc3339_to_unix("2026-07-06T05:00:00-05:00"), 1783332000);
        assert_eq!(rfc3339_to_unix("2026-07-06T10:00:00.123456789Z"), 1783332000);
        assert_eq!(rfc3339_to_unix("2026-07-06T12:00:00+0200"), 1783332000);
        assert_eq!(rfc3339_to_unix("2026-07-06T12:00:00+02"), 1783332000);
        // Without offset suffix, assume UTC
        assert_eq!(rfc3339_to_unix("2026-07-06T10:00:00"), 1783332000);
    }

    #[test]
    fn rfc3339_to_unix_never_panics_on_malformed_tz_suffix() {
        // Non-ASCII bytes in suffix: should not panic, treat as UTC (offset 0)
        assert_eq!(rfc3339_to_unix("2026-07-06T10:00:00+1à"), 1783332000);
        // Garbage timezone: should not panic, treat as UTC
        assert_eq!(rfc3339_to_unix("2026-07-06T10:00:00+xx:zz"), 1783332000);
    }

    #[test]
    fn parse_lsjson_rejects_garbage() {
        assert!(parse_lsjson("not json").is_err());
    }

    #[test]
    fn identity_formats_per_provider() {
        assert_eq!(
            identity("sftp", Some("nas.local:22"), Some("milosz"), Some("backups")),
            "sftp:milosz@nas.local:22/backups");
        assert_eq!(identity("dropbox", None, None, None), "dropbox/");
    }
}
