/// Build ffmpeg args for a keyframe-snapped stream-copy trim (the "Fast (lossless)" path).
/// Input-side `-ss` does a fast seek; `-t` bounds the copied duration. With `-c copy`
/// the start snaps to the nearest preceding keyframe, so output duration is within one
/// GOP of the requested span.
pub fn build_trim_copy_args(input: &str, in_secs: f64, out_secs: f64, output: &str) -> Vec<String> {
    let duration = out_secs - in_secs;
    vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
        "-c".to_string(),
        "copy".to_string(),
        output.to_string(),
    ]
}

/// Validate a trim request before it is handed to ffmpeg.
/// Reference-in-place: the output must never be the source file (with `-y -c copy`
/// ffmpeg would truncate the source while reading it). The span must be ordered.
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
            vec![
                "-y",
                "-ss", "1.000",
                "-i", "in.mp4",
                "-t", "2.500",
                "-c", "copy",
                "out.mp4",
            ]
        );
    }

    #[test]
    fn validate_trim_rejects_output_equal_to_source() {
        let result = validate_trim("clip.mp4", "clip.mp4", 1.0, 3.5);
        assert!(result.is_err());
    }

    #[test]
    fn validate_trim_rejects_in_after_or_equal_out() {
        let equal = validate_trim("in.mp4", "out.mp4", 2.0, 2.0);
        assert!(equal.is_err());

        let reversed = validate_trim("in.mp4", "out.mp4", 5.0, 2.0);
        assert!(reversed.is_err());
    }

    #[test]
    fn validate_trim_accepts_valid_distinct_ordered_span() {
        let result = validate_trim("in.mp4", "out.mp4", 1.0, 3.5);
        assert!(result.is_ok());
    }
}
