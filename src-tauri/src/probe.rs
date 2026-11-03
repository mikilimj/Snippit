use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMeta {
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub container: String,
    pub video_codec: String,
    pub audio_tracks: Vec<AudioTrack>,
    /// When the footage was recorded, as a Unix timestamp in seconds. Sourced
    /// from the container's own metadata (`format.tags` / stream tags), NOT the
    /// file mtime — a copied or re-located file keeps its original capture time
    /// while its mtime becomes the copy date. `None` when the file carries no
    /// capture metadata at all, in which case the library falls back to the
    /// file's mtime.
    #[serde(default)]
    pub captured_at: Option<i64>,
    /// The source file's mtime at probe time, in Unix seconds. Stored so the
    /// library can order clips that carry no capture metadata without touching the
    /// filesystem on every list query. Filled in by `probe_path`, which is the only
    /// place with the path; parsers that only see ffprobe JSON leave it `None`.
    #[serde(default)]
    pub file_mtime: Option<i64>,
}

pub fn parse_ffprobe_json(json: &str) -> Result<ClipMeta, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let streams = v["streams"].as_array().ok_or("no streams in ffprobe output")?;

    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or("no video stream found")?;
    let width = video["width"].as_u64().ok_or("video stream missing width")? as u32;
    let height = video["height"].as_u64().ok_or("video stream missing height")? as u32;
    let video_codec = video["codec_name"].as_str().unwrap_or("unknown").to_string();

    let audio_tracks = streams
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .map(|s| AudioTrack {
            index: s["index"].as_u64().unwrap_or(0) as u32,
            codec: s["codec_name"].as_str().unwrap_or("unknown").to_string(),
            channels: s["channels"].as_u64().unwrap_or(0) as u32,
            language: s["tags"]["language"].as_str().map(|x| x.to_string()),
        })
        .collect();

    let container = v["format"]["format_name"].as_str().unwrap_or("unknown").to_string();
    let duration_secs = v["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .ok_or("format.duration missing or unparseable")?;

    // Prefer the container's own mux-level tag, then the video stream's, then any
    // other A/V stream's — recorders disagree on where they write it. Subtitle and
    // attachment streams are skipped: muxers often write the *mux* time there,
    // which would read as a capture time.
    let captured_at = find_capture_time(&v["format"]["tags"])
        .or_else(|| find_capture_time(&video["tags"]))
        .or_else(|| {
            streams
                .iter()
                .filter(|s| s["codec_type"] == "video" || s["codec_type"] == "audio")
                .find_map(|s| find_capture_time(&s["tags"]))
        });

    Ok(ClipMeta {
        duration_secs,
        width,
        height,
        container,
        video_codec,
        audio_tracks,
        captured_at,
        file_mtime: None,
    })
}

/// Tag keys that carry a recording timestamp, in the order we trust them. The
/// Apple key matters for iPhone/iPad footage, `date` for some AVI/MXF muxers.
const CAPTURE_TIME_KEYS: [&str; 4] = [
    "creation_time",
    "com.apple.quicktime.creationdate",
    "date",
    "creationdate",
];

/// Reads a capture timestamp out of an ffprobe `tags` object, if present.
fn find_capture_time(tags: &serde_json::Value) -> Option<i64> {
    let obj = tags.as_object()?;
    for key in CAPTURE_TIME_KEYS {
        let hit = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key));
        if let Some((_, v)) = hit {
            if let Some(ts) = v.as_str().and_then(parse_capture_time) {
                return Some(ts);
            }
        }
    }
    None
}

/// Parses the timestamp formats containers expose into Unix seconds. Accepts
/// ISO 8601 / RFC 3339 (`2026-11-03T14:22:01.123456Z`, `+02:00` offsets, or a
/// space instead of `T`) and date-only `2026-11-03`. Zone-less timestamps are
/// read as UTC, matching how FFmpeg writes `creation_time`. Anything else yields
/// `None` — a missing capture time must never block ingestion or re-probing.
pub fn parse_capture_time(raw: &str) -> Option<i64> {
    let s = raw.trim();
    let (date, rest) = s.split_at_checked(10)?;
    if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return None;
    }
    let year: i64 = date[0..4].parse().ok()?;
    let month: i64 = date[5..7].parse().ok()?;
    let day: i64 = date[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let mut secs = days_from_civil(year, month, day) * 86_400;

    // Date-only tags (some AVI/MKV muxers) carry midnight implicitly.
    let rest = rest.trim_start_matches(['T', 't', ' ']);
    if rest.is_empty() {
        return Some(secs);
    }

    let (hms, zone) = match rest.find(['Z', 'z', '+']) {
        Some(i) => (&rest[..i], &rest[i..]),
        // A '-' can only start a negative offset after the time, so look past it.
        // `get` keeps a truncated tag (`2026-11-03T14`) a parse failure, not a panic:
        // slicing `rest[8..]` on a short time part aborted the probe instead.
        None => match rest.get(8..).and_then(|tail| tail.find('-').map(|i| i + 8)) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        },
    };
    let mut parts = hms.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let sec: i64 = parts.next().unwrap_or("0").split('.').next()?.parse().ok()?;
    if h > 23 || m > 59 || sec > 60 {
        return None;
    }
    secs += h * 3_600 + m * 60 + sec;

    if let Some(off) = zone.strip_prefix(['-', '+']) {
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        // Containers emit both `+01:00` and the ISO 8601 basic form `+0100`.
        // `get` rather than slicing: the offset comes from container metadata and
        // may contain non-ASCII bytes, which would split a char boundary.
        let (oh, om): (i64, i64) = match off.split_once(':') {
            Some((h, m)) => (h.parse().ok()?, m.parse().ok()?),
            None if off.len() == 4 => (off.get(..2)?.parse().ok()?, off.get(2..)?.parse().ok()?),
            None if off.len() == 2 => (off.parse().ok()?, 0),
            None => return None,
        };
        secs -= sign * (oh * 3_600 + om * 60);
    }
    Some(secs)
}

/// Days in a month, leap years included. Without it an impossible tag like
/// `2026-02-31` is silently normalised into March instead of being rejected.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`). Kept local so capture-time parsing needs no date crate.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "streams": [
        { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080 },
        { "index": 1, "codec_type": "audio", "codec_name": "aac", "channels": 2, "tags": { "language": "eng" } },
        { "index": 2, "codec_type": "audio", "codec_name": "aac", "channels": 2 }
      ],
      "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.345" }
    }"#;

    #[test]
    fn parses_video_audio_and_format() {
        let m = parse_ffprobe_json(SAMPLE).unwrap();
        assert_eq!(m.width, 1920);
        assert_eq!(m.height, 1080);
        assert_eq!(m.video_codec, "h264");
        assert_eq!(m.container, "mov,mp4,m4a,3gp,3g2,mj2");
        assert!((m.duration_secs - 12.345).abs() < 1e-6);
        assert_eq!(m.audio_tracks.len(), 2);
        assert_eq!(m.audio_tracks[0].language.as_deref(), Some("eng"));
        assert_eq!(m.audio_tracks[1].language, None);
        assert_eq!(m.audio_tracks[0].channels, 2);
        assert_eq!(m.captured_at, None, "no capture tag in this sample");
    }

    #[test]
    fn reads_capture_time_from_the_mux_level_tag() {
        let json = r#"{
          "streams": [
            { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 8, "height": 8 }
          ],
          "format": {
            "format_name": "mp4",
            "duration": "1.0",
            "tags": { "creation_time": "2026-11-03T14:22:01.123456Z" }
          }
        }"#;
        let m = parse_ffprobe_json(json).unwrap();
        // Cross-checked against `date -d` in the parser tests below.
        assert_eq!(m.captured_at, Some(1_793_715_721));
    }

    #[test]
    fn reads_capture_time_from_a_stream_tag_when_the_mux_has_none() {
        // Some screen recorders write the timestamp onto the video stream only.
        let json = r#"{
          "streams": [
            { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 8, "height": 8,
              "tags": { "creation_time": "2024-01-02T03:04:05Z" } }
          ],
          "format": { "format_name": "mp4", "duration": "1.0" }
        }"#;
        assert_eq!(parse_ffprobe_json(json).unwrap().captured_at, Some(1_704_164_645));
    }

    #[test]
    fn the_apple_quicktime_key_is_honoured() {
        // iPhone footage carries `com.apple.quicktime.creationdate` with an offset.
        let json = r#"{
          "streams": [
            { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 8, "height": 8 }
          ],
          "format": { "format_name": "mov", "duration": "1.0",
            "tags": { "com.apple.quicktime.creationdate": "2026-11-03T15:22:01+0100" } }
        }"#;
        // Same instant as the UTC-mid-day sample above, expressed at +01:00.
        assert_eq!(parse_ffprobe_json(json).unwrap().captured_at, Some(1_793_715_721));
    }

    #[test]
    fn unparseable_capture_tags_yield_none_instead_of_failing_the_probe() {
        // A junk tag must never make an otherwise playable clip unindexable.
        let json = r#"{
          "streams": [
            { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 8, "height": 8 }
          ],
          "format": { "format_name": "mp4", "duration": "1.0",
            "tags": { "creation_time": "not a date" } }
        }"#;
        let m = parse_ffprobe_json(json).unwrap();
        assert_eq!(m.captured_at, None);
        assert_eq!(m.duration_secs, 1.0);
    }

    #[test]
    fn parse_capture_time_accepts_the_formats_containers_emit() {
        let utc = parse_capture_time("2026-11-03T14:22:01Z").unwrap();
        // Fractional seconds are truncated, not rounded.
        assert_eq!(parse_capture_time("2026-11-03T14:22:01.999Z").unwrap(), utc);
        // A space instead of `T` (AVI/MKV `date` tags).
        assert_eq!(parse_capture_time("2026-11-03 14:22:01").unwrap(), utc);
        // Explicit offsets shift back to UTC.
        assert_eq!(parse_capture_time("2026-11-03T15:22:01+01:00").unwrap(), utc);
        assert_eq!(parse_capture_time("2026-11-03T14:22:01-00:00").unwrap(), utc);
        assert_eq!(parse_capture_time("2026-11-03T09:22:01-05:00").unwrap(), utc);
        // The ISO 8601 basic offset form, which Apple's tag uses.
        assert_eq!(parse_capture_time("2026-11-03T15:22:01+0100").unwrap(), utc);
        assert_eq!(parse_capture_time("2026-11-03T09:22:01-0500").unwrap(), utc);
        assert_eq!(parse_capture_time("2026-11-03T15:22:01+01").unwrap(), utc);
        // Date-only tags mean midnight UTC.
        assert_eq!(
            parse_capture_time("2026-11-03").unwrap(),
            utc - (14 * 3_600 + 22 * 60 + 1)
        );
    }

    #[test]
    fn parse_capture_time_rejects_garbage() {
        for junk in [
            "",
            "unknown",
            "0000-00-00 00:00:00",
            "2026-13-01T00:00:00Z", // month 13
            "2026-11-03T25:00:00Z", // hour 25
            "2026-11-03T14:99:00Z", // minute 99
            "2026-02-31T00:00:00Z", // Feb 31 — would otherwise slide into March
            "2025-02-29T00:00:00Z", // 2025 is not a leap year
            "20261103",             // no separators
        ] {
            assert_eq!(parse_capture_time(junk), None, "should reject {junk:?}");
        }
        // The leap day of an actual leap year still parses.
        assert!(parse_capture_time("2024-02-29T12:00:00Z").is_some());
    }

    #[test]
    fn truncated_time_parts_are_rejected_without_panicking() {
        // Regression: the negative-offset probe sliced `rest[8..]`, so any tag
        // whose time part was shorter than `HH:MM:SS` aborted the whole probe
        // instead of yielding `None` — one bad tag made a clip unindexable.
        let utc = parse_capture_time("2026-11-03T14:22:01Z").unwrap();
        for junk in ["2026-11-03T14", "2026-11-03T1"] {
            assert_eq!(parse_capture_time(junk), None, "should reject {junk:?}");
        }
        // Shorter but structurally complete times still parse, as before: seconds
        // are optional, and an unpadded hour is accepted.
        assert_eq!(parse_capture_time("2026-11-03T14:22").unwrap(), utc - 1);
        assert_eq!(parse_capture_time("2026-11-03 8:22:01").unwrap(), utc - 6 * 3_600);
        assert!(parse_capture_time("2026-11-03 8:00").is_some());
    }

    #[test]
    fn a_non_ascii_offset_is_rejected_without_panicking() {
        // Regression: `off[..2]` on container-supplied bytes could split a char
        // boundary. Metadata is untrusted input; it must never panic.
        assert_eq!(parse_capture_time("2026-11-03T14:22:01+\u{4e00}2"), None);
        assert_eq!(parse_capture_time("2026-11-03T14:22:01+\u{e9}1:00"), None);
    }

    #[test]
    fn parse_capture_time_handles_dates_before_and_after_the_epoch() {
        assert_eq!(parse_capture_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_capture_time("1969-12-31T23:59:59Z"), Some(-1));
        // 2000-03-01 straddles a leap day — the classic off-by-one in date math.
        assert_eq!(parse_capture_time("2000-03-01T00:00:00Z"), Some(951_868_800));
    }
}
