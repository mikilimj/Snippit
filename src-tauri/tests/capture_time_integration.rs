//! Capture-time extraction against real ffprobe output.
//!
//! The parser unit tests feed hand-written JSON; these exercise the formats that
//! ffmpeg actually writes, which is what decides whether a recorder's footage
//! sorts by recording date or falls back to file mtime.

use std::process::Command;

use snippit_lib::probe::{parse_capture_time, parse_ffprobe_json};

/// Runs real ffprobe on `path` and parses the result the way `probe_path` does.
fn probe(path: &str) -> snippit_lib::probe::ClipMeta {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", path])
        .output()
        .expect("ffprobe runs");
    assert!(out.status.success(), "ffprobe failed: {}", String::from_utf8_lossy(&out.stderr));
    parse_ffprobe_json(&String::from_utf8_lossy(&out.stdout)).expect("parses")
}

/// Generates a clip, optionally stamping it with `creation_time`.
fn make_clip(path: &str, creation_time: Option<&str>) {
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-y", "-v", "error",
        "-f", "lavfi", "-i", "testsrc=duration=1:size=64x64:rate=5",
    ]);
    if let Some(ct) = creation_time {
        cmd.args(["-metadata", &format!("creation_time={ct}")]);
    }
    cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", path]);
    let status = cmd.status().expect("ffmpeg runs");
    assert!(status.success(), "clip generation failed");
}

#[test]
fn reads_creation_time_that_ffmpeg_actually_writes() {
    let path = std::env::temp_dir().join("snippit_captime_tagged.mp4");
    let path_s = path.to_str().unwrap();
    make_clip(path_s, Some("2026-11-03T14:22:01Z"));

    let meta = probe(path_s);
    // ffmpeg re-writes the tag as `2026-11-03T14:22:01.000000Z`; the parser must
    // cope with the fractional form and land on the same instant.
    assert_eq!(meta.captured_at, Some(1_793_715_721));
    assert_eq!(meta.captured_at, parse_capture_time("2026-11-03T14:22:01Z"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_clip_without_creation_time_reports_none_so_mtime_can_take_over() {
    let path = std::env::temp_dir().join("snippit_captime_untagged.mp4");
    let path_s = path.to_str().unwrap();
    make_clip(path_s, None);

    let meta = probe(path_s);
    // Not an error: this is the common screen-recorder case, and the library
    // falls back to the file's mtime for ordering.
    assert_eq!(meta.captured_at, None);
    assert!(meta.duration_secs > 0.0, "the rest of the metadata still parses");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn capture_time_reflects_the_recording_instant_not_the_copy_time() {
    // The whole point of using container metadata: a file copied long after it
    // was recorded keeps its capture date even though its mtime is "now".
    let path = std::env::temp_dir().join("snippit_captime_old.mp4");
    let path_s = path.to_str().unwrap();
    make_clip(path_s, Some("2019-06-01T08:00:00Z"));

    let meta = probe(path_s);
    let mtime = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64)
        .unwrap();
    assert_eq!(meta.captured_at, parse_capture_time("2019-06-01T08:00:00Z"));
    assert!(
        meta.captured_at.unwrap() < mtime,
        "capture time ({:?}) should predate the mtime ({mtime})",
        meta.captured_at
    );

    let _ = std::fs::remove_file(&path);
}
