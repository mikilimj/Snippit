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

pub fn validate_trim(source: &str, output: &str, in_secs: f64, out_secs: f64) -> Result<(), String> {
    if output == source {
        return Err("Output path must differ from the source file.".into());
    }
    if !(in_secs < out_secs) {
        return Err("In point must be before out point.".into());
    }
    Ok(())
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
}
