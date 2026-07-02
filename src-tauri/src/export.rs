use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackConfig {
    pub stream_index: u32,
    pub enabled: bool,
    pub volume: f64,
}

/// Build ffmpeg args for a trim export with per-track audio control.
///
/// Fast path (stream copy): exactly one enabled track, at volume ≈ 1.0.
/// Mix path (re-encode audio): two or more enabled tracks (always mixed into a single
/// output stream, even at unity volume — a multi-stream container would leave every
/// player but the first track silent) or a single track that deviates from unity.
pub fn build_trim_args(
    input: &str,
    in_secs: f64,
    out_secs: f64,
    output: &str,
    tracks: &[TrackConfig],
) -> Vec<String> {
    let duration = out_secs - in_secs;
    let mut args = vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
    ];

    let enabled: Vec<&TrackConfig> = tracks.iter().filter(|t| t.enabled).collect();

    if enabled.is_empty() {
        // No explicit config or all muted → copy all streams as before
        args.extend(["-c".to_string(), "copy".to_string(), output.to_string()]);
        return args;
    }

    let single_track_unity =
        enabled.len() == 1 && (enabled[0].volume - 1.0).abs() < 0.001;

    if single_track_unity {
        // Fast path: only one audio stream in the output, so stream-copy is safe —
        // there's nothing to mix and no other track for a player to ignore.
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
        args.extend(["-map".to_string(), format!("0:{}", enabled[0].stream_index)]);
        args.extend(["-c".to_string(), "copy".to_string(), output.to_string()]);
        return args;
    }

    // Mix path: volume filter(s) + amix, re-encode audio
    let filter = amix_filter(&enabled);
    args.extend([
        "-filter_complex".to_string(),
        filter,
        "-map".to_string(),
        "0:v:0".to_string(),
        "-map".to_string(),
        "[aout]".to_string(),
        "-c:v".to_string(),
        "copy".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        output.to_string(),
    ]);
    args
}

fn amix_filter(enabled: &[&TrackConfig]) -> String {
    if enabled.len() == 1 {
        return format!("[0:{}]volume={:.3}[aout]", enabled[0].stream_index, enabled[0].volume);
    }
    let mut parts: Vec<String> = enabled
        .iter()
        .enumerate()
        .map(|(i, t)| format!("[0:{}]volume={:.3}[a{i}]", t.stream_index, t.volume))
        .collect();
    let labels: String = (0..enabled.len()).map(|i| format!("[a{i}]")).collect();
    parts.push(format!(
        "{}amix=inputs={}:duration=first:normalize=0[aout]",
        labels,
        enabled.len()
    ));
    parts.join(";")
}

/// Legacy wrapper kept for tests. Equivalent to build_trim_args with no track overrides
/// (copies all streams unchanged).
pub fn build_trim_copy_args(input: &str, in_secs: f64, out_secs: f64, output: &str) -> Vec<String> {
    build_trim_args(input, in_secs, out_secs, output, &[])
}

/// Build ffmpeg args for a **Compatible MP4** trim export (spec §4.5): the video
/// is always re-encoded to H.264 (and audio to AAC) for universal playback and
/// frame-accurate cut points, rather than stream-copied at keyframes. Audio track
/// mapping/mixing follows the same rules as `build_trim_args`.
pub fn build_trim_compatible_args(
    input: &str,
    in_secs: f64,
    out_secs: f64,
    output: &str,
    tracks: &[TrackConfig],
) -> Vec<String> {
    let duration = out_secs - in_secs;
    let mut args = vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
    ];

    let enabled: Vec<&TrackConfig> = tracks.iter().filter(|t| t.enabled).collect();
    let needs_mix =
        enabled.len() > 1 || (enabled.len() == 1 && (enabled[0].volume - 1.0).abs() >= 0.001);

    if needs_mix {
        args.extend([
            "-filter_complex".to_string(),
            amix_filter(&enabled),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "[aout]".to_string(),
        ]);
    } else if enabled.len() == 1 {
        args.extend([
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            format!("0:{}", enabled[0].stream_index),
        ]);
    } else if !tracks.is_empty() {
        // Tracks configured but all muted → video only.
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
    }
    // (empty `tracks` = no audio config → let ffmpeg map defaults)

    args.extend([
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        "veryfast".to_string(),
        "-crf".to_string(),
        "20".to_string(),
    ]);

    let audio_silenced = !tracks.is_empty() && enabled.is_empty();
    if audio_silenced {
        args.push("-an".to_string());
    } else {
        args.extend([
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            "192k".to_string(),
        ]);
    }

    args.extend([
        "-movflags".to_string(),
        "+faststart".to_string(),
        output.to_string(),
    ]);
    args
}

/// Parses one ffmpeg `-progress` line into a 0.0–1.0 completion fraction against
/// `total_secs`. `out_time_us` is microseconds; `progress=end` marks completion.
/// Returns None for lines without a usable timestamp (e.g. `frame=…`, `N/A`).
pub fn parse_progress_line(line: &str, total_secs: f64) -> Option<f64> {
    let line = line.trim();
    if line == "progress=end" {
        return Some(1.0);
    }
    let micros: i64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    if total_secs <= 0.0 {
        return Some(0.0);
    }
    Some(((micros as f64 / 1_000_000.0) / total_secs).clamp(0.0, 1.0))
}

pub fn validate_trim(source: &str, output: &str, in_secs: f64, out_secs: f64) -> Result<(), String> {
    if output == source {
        return Err("Output path must differ from the source file.".into());
    }
    if !(in_secs < out_secs) {
        return Err("In point must be before out point.".into());
    }
    Ok(())
}

// ── Target-size compression (design spec §4.5) ────────────────────────────────────

/// Below this the picture is unusable at any resolution a game clip ships in.
pub const MIN_VIDEO_KBPS: u32 = 100;

/// Fraction of the target reserved for container overhead and rate-control drift.
const SIZE_SAFETY: f64 = 0.97;

/// Split a target file size into (video_kbps, audio_kbps) for a two-pass encode.
///
/// `target_total_bits = target_mb × 8 × 1024 × 1024 × safety`; audio gets a fixed
/// bitrate (stepped down for tight budgets) and video gets the remainder. Errors
/// when the remainder falls below MIN_VIDEO_KBPS — the pre-flight warning from the
/// design spec — so the user can shorten the clip or pick a larger target instead
/// of burning a two-pass encode on an unwatchable file.
pub fn compute_target_bitrates(target_mb: f64, duration_secs: f64) -> Result<(u32, u32), String> {
    if target_mb <= 0.0 {
        return Err("Target size must be greater than zero.".into());
    }
    if duration_secs <= 0.0 {
        return Err("Clip duration must be greater than zero.".into());
    }
    let total_kbps = target_mb * 8.0 * 1024.0 * 1024.0 * SIZE_SAFETY / 1000.0 / duration_secs;
    let audio_kbps: u32 = if total_kbps >= 512.0 {
        128
    } else if total_kbps >= 256.0 {
        96
    } else {
        64
    };
    let video_kbps = total_kbps - audio_kbps as f64;
    if video_kbps < MIN_VIDEO_KBPS as f64 {
        return Err(format!(
            "{target_mb} MB is too small for a {duration_secs:.1}s clip — video would drop below {MIN_VIDEO_KBPS} kbps. Shorten the clip or choose a bigger target."
        ));
    }
    Ok((video_kbps.floor() as u32, audio_kbps))
}

fn null_sink() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

/// Build ffmpeg args for one pass of a two-pass H.264/AAC target-size encode.
///
/// Pass 1 analyzes only (video discarded to the OS null sink, audio dropped);
/// pass 2 encodes the real output, applying the same per-track audio semantics
/// as `build_trim_args` except audio is always re-encoded to AAC — stream copy
/// can't hit a bitrate target.
pub fn build_compress_pass_args(
    input: &str,
    in_secs: f64,
    out_secs: f64,
    output: &str,
    tracks: &[TrackConfig],
    video_kbps: u32,
    audio_kbps: u32,
    pass: u8,
    passlog: &str,
) -> Vec<String> {
    let duration = out_secs - in_secs;
    let mut args = vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
    ];

    let enabled: Vec<&TrackConfig> = tracks.iter().filter(|t| t.enabled).collect();

    if pass == 1 {
        args.extend([
            "-map".to_string(),
            "0:v:0".to_string(),
            "-c:v".to_string(),
            "libx264".to_string(),
            "-b:v".to_string(),
            format!("{video_kbps}k"),
            "-preset".to_string(),
            "medium".to_string(),
            "-pass".to_string(),
            "1".to_string(),
            "-passlogfile".to_string(),
            passlog.to_string(),
            "-an".to_string(),
            "-f".to_string(),
            "mp4".to_string(),
            null_sink().to_string(),
        ]);
        return args;
    }

    // Pass 2: audio mapping mirrors build_trim_args' mix/single-track semantics.
    let needs_mix = enabled.len() > 1
        || (enabled.len() == 1 && (enabled[0].volume - 1.0).abs() >= 0.001);
    if needs_mix {
        args.extend([
            "-filter_complex".to_string(),
            amix_filter(&enabled),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "[aout]".to_string(),
        ]);
    } else if enabled.len() == 1 {
        args.extend([
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            format!("0:{}", enabled[0].stream_index),
        ]);
    } else if !tracks.is_empty() {
        // Tracks were configured but every one is muted → video only.
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
    }
    // (empty `tracks` = clip with no audio config → let ffmpeg pick defaults)

    args.extend([
        "-c:v".to_string(),
        "libx264".to_string(),
        "-b:v".to_string(),
        format!("{video_kbps}k"),
        "-preset".to_string(),
        "medium".to_string(),
        "-pass".to_string(),
        "2".to_string(),
        "-passlogfile".to_string(),
        passlog.to_string(),
    ]);

    let audio_silenced = !tracks.is_empty() && enabled.is_empty();
    if audio_silenced {
        args.push("-an".to_string());
    } else {
        args.extend([
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            format!("{audio_kbps}k"),
        ]);
    }

    args.extend([
        "-movflags".to_string(),
        "+faststart".to_string(),
        output.to_string(),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_input_seek_copy_args() {
        let args = build_trim_copy_args("in.mp4", 1.0, 3.5, "out.mp4");
        assert_eq!(
            args,
            vec!["-y", "-ss", "1.000", "-i", "in.mp4", "-t", "2.500", "-c", "copy", "out.mp4"]
        );
    }

    #[test]
    fn build_trim_args_single_enabled_track_at_unity_uses_stream_copy() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(args.contains(&"0:v:0".to_string()));
        assert!(args.contains(&"0:1".to_string()));
        assert!(args.contains(&"copy".to_string()));
        assert!(!args.contains(&"-filter_complex".to_string()));
    }

    #[test]
    fn build_trim_args_multiple_unity_tracks_still_mixes_to_one_stream() {
        // Two tracks both at 100% must still be combined into a single output
        // audio stream via amix — a multi-map stream-copy would leave every
        // track but the first silent in players that don't expose track pickers.
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 1.0 },
        ];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(args.contains(&"-filter_complex".to_string()));
        let fc_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[fc_idx + 1];
        assert!(filter.contains("amix=inputs=2"));
        assert!(filter.contains("[0:1]volume=1.000"));
        assert!(filter.contains("[0:2]volume=1.000"));
        assert!(args.contains(&"[aout]".to_string()));
        assert!(!args.contains(&"0:1".to_string())); // not stream-copy-mapped raw
    }

    #[test]
    fn build_trim_args_non_unity_volume_uses_amix() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 0.5 },
        ];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(args.contains(&"-filter_complex".to_string()));
        let fc_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[fc_idx + 1];
        assert!(filter.contains("amix=inputs=2"));
        assert!(filter.contains("volume=0.500"));
        assert!(args.contains(&"aac".to_string()));
    }

    #[test]
    fn build_trim_args_single_track_non_unity_no_amix() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 0.7 }];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        let fc_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[fc_idx + 1];
        assert!(filter.contains("[aout]"));
        assert!(!filter.contains("amix"));
    }

    #[test]
    fn build_trim_args_muted_track_excluded() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: false, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 1.0 },
        ];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(!args.contains(&"0:1".to_string()));
        assert!(args.contains(&"0:2".to_string()));
    }

    #[test]
    fn validate_trim_rejects_output_equal_to_source() {
        assert!(validate_trim("clip.mp4", "clip.mp4", 1.0, 3.5).is_err());
    }

    #[test]
    fn validate_trim_rejects_in_after_or_equal_out() {
        assert!(validate_trim("in.mp4", "out.mp4", 2.0, 2.0).is_err());
        assert!(validate_trim("in.mp4", "out.mp4", 5.0, 2.0).is_err());
    }

    #[test]
    fn validate_trim_accepts_valid_distinct_ordered_span() {
        assert!(validate_trim("in.mp4", "out.mp4", 1.0, 3.5).is_ok());
    }

    // ── Target-size compression ──────────────────────────────────────────────────

    #[test]
    fn compute_target_bitrates_splits_budget_between_video_and_audio() {
        // 10 MB over 60s: total ≈ 1356 kbps → 128k audio, remainder to video.
        let (v, a) = compute_target_bitrates(10.0, 60.0).unwrap();
        assert_eq!(a, 128);
        assert!((1200..=1240).contains(&v), "video kbps out of range: {v}");
    }

    #[test]
    fn compute_target_bitrates_steps_audio_down_for_tight_budgets() {
        // 8 MB over 300s: total ≈ 217 kbps → audio must drop to 64k.
        let (v, a) = compute_target_bitrates(8.0, 300.0).unwrap();
        assert_eq!(a, 64);
        assert!(v >= MIN_VIDEO_KBPS);
    }

    #[test]
    fn compute_target_bitrates_errors_below_video_floor() {
        // 8 MB over an hour is hopeless — must refuse, not encode garbage.
        let err = compute_target_bitrates(8.0, 3600.0).unwrap_err();
        assert!(err.contains("too small"), "unexpected message: {err}");
    }

    #[test]
    fn compute_target_bitrates_rejects_non_positive_inputs() {
        assert!(compute_target_bitrates(0.0, 60.0).is_err());
        assert!(compute_target_bitrates(10.0, 0.0).is_err());
    }

    #[test]
    fn compress_pass1_analyzes_video_only_into_null_sink() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args =
            build_compress_pass_args("in.mp4", 1.0, 11.0, "out.mp4", &tracks, 1200, 128, 1, "/tmp/log");
        assert!(args.contains(&"-pass".to_string()));
        let p = args.iter().position(|a| a == "-pass").unwrap();
        assert_eq!(args[p + 1], "1");
        assert!(args.contains(&"-an".to_string()));
        assert!(args.contains(&"1200k".to_string()));
        assert!(!args.contains(&"out.mp4".to_string()));
        assert!(args.last().unwrap() == "/dev/null" || args.last().unwrap() == "NUL");
    }

    #[test]
    fn compress_pass2_single_unity_track_maps_stream_and_reencodes_aac() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args =
            build_compress_pass_args("in.mp4", 1.0, 11.0, "out.mp4", &tracks, 1200, 128, 2, "/tmp/log");
        assert!(args.contains(&"0:1".to_string()));
        assert!(!args.contains(&"-filter_complex".to_string()));
        assert!(args.contains(&"aac".to_string()));
        assert!(args.contains(&"128k".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
        // Video must be re-encoded to hit the target — never stream-copied.
        assert!(args.contains(&"libx264".to_string()));
        assert!(!args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "copy"));
    }

    #[test]
    fn compress_pass2_multiple_tracks_use_amix() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 0.5 },
        ];
        let args =
            build_compress_pass_args("in.mp4", 0.0, 10.0, "out.mp4", &tracks, 900, 96, 2, "/tmp/log");
        let fc = args.iter().position(|a| a == "-filter_complex").unwrap();
        assert!(args[fc + 1].contains("amix=inputs=2"));
        assert!(args.contains(&"[aout]".to_string()));
        assert!(args.contains(&"96k".to_string()));
    }

    #[test]
    fn compress_pass2_all_tracks_muted_drops_audio() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: false, volume: 1.0 }];
        let args =
            build_compress_pass_args("in.mp4", 0.0, 10.0, "out.mp4", &tracks, 900, 128, 2, "/tmp/log");
        assert!(args.contains(&"-an".to_string()));
        assert!(!args.contains(&"aac".to_string()));
    }

    // ── Compatible (re-encode) export mode ───────────────────────────────────────

    #[test]
    fn compatible_reencodes_video_to_h264_with_faststart() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args = build_trim_compatible_args("in.mkv", 1.0, 6.0, "out.mp4", &tracks);
        // Even a single unity track must re-encode video for universal playback.
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
        assert!(!args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "copy"));
        assert!(args.contains(&"aac".to_string())); // audio re-encoded too
        assert!(args.windows(2).any(|w| w[0] == "-movflags" && w[1] == "+faststart"));
        assert_eq!(args.last().unwrap(), "out.mp4");
        // Trim window preserved.
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[ss + 1], "1.000");
        assert!(args.windows(2).any(|w| w[0] == "-t" && w[1] == "5.000"));
    }

    #[test]
    fn compatible_mixes_multiple_tracks() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 0.5 },
        ];
        let args = build_trim_compatible_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        let fc = args.iter().position(|a| a == "-filter_complex").unwrap();
        assert!(args[fc + 1].contains("amix=inputs=2"));
        assert!(args.contains(&"[aout]".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
    }

    #[test]
    fn compatible_all_muted_drops_audio_but_still_reencodes_video() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: false, volume: 1.0 }];
        let args = build_trim_compatible_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(args.contains(&"-an".to_string()));
        assert!(!args.contains(&"aac".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
    }

    // ── FFmpeg -progress parsing ─────────────────────────────────────────────────

    #[test]
    fn progress_line_reports_fraction_of_duration() {
        // Halfway through a 10s job.
        let f = parse_progress_line("out_time_us=5000000", 10.0).unwrap();
        assert!((f - 0.5).abs() < 1e-6, "got {f}");
    }

    #[test]
    fn progress_end_marker_is_complete() {
        assert_eq!(parse_progress_line("progress=end", 10.0), Some(1.0));
    }

    #[test]
    fn progress_line_clamps_and_ignores_irrelevant_lines() {
        // Overshoot (encoder can report slightly past duration) clamps to 1.0.
        assert_eq!(parse_progress_line("out_time_us=11000000", 10.0), Some(1.0));
        // Non-timestamp lines yield nothing.
        assert_eq!(parse_progress_line("frame=42", 10.0), None);
        assert_eq!(parse_progress_line("out_time_us=N/A", 10.0), None);
    }
}
