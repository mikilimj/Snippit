use std::process::Command;

use snippit_lib::export::build_trim_copy_args;

fn probe_duration(path: &str) -> f64 {
    let out = Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            path,
        ])
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&out.stdout).trim().parse().expect("duration parses")
}

#[test]
fn trims_a_generated_clip_to_requested_span() {
    let dir = std::env::temp_dir();
    let src = dir.join("snippit_src.mp4");
    let dst = dir.join("snippit_dst.mp4");
    let src_s = src.to_str().unwrap();
    let dst_s = dst.to_str().unwrap();

    // Generate a 6s 320x240 clip with a small GOP so keyframe-snapped trim is near-exact.
    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=6:size=320x240:rate=30",
            "-f", "lavfi", "-i", "sine=frequency=1000:duration=6",
            "-c:v", "libx264", "-g", "15", "-pix_fmt", "yuv420p",
            "-c:a", "aac",
            src_s,
        ])
        .status()
        .expect("ffmpeg generate runs");
    assert!(gen.success(), "clip generation failed");

    // Trim 1.0 .. 3.0 -> expect ~2.0s.
    let trim = Command::new("ffmpeg")
        .args(build_trim_copy_args(src_s, 1.0, 3.0, dst_s))
        .status()
        .expect("ffmpeg trim runs");
    assert!(trim.success(), "trim failed");

    let d = probe_duration(dst_s);
    assert!((d - 2.0).abs() < 0.5, "expected ~2.0s, got {d}");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
}
