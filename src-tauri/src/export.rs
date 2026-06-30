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
}
