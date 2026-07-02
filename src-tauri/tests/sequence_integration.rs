use std::process::Command;

use clipsnip_lib::export::{build_sequence_args, MusicOverlay, Segment};

fn gen_clip(path: &str, size: &str, freq: u32) {
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", &format!("testsrc=duration=2:size={size}:rate=30"),
            "-f", "lavfi", "-i", &format!("sine=frequency={freq}:duration=2"),
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
            path,
        ])
        .status()
        .expect("ffmpeg runs");
    assert!(ok.success(), "clip generation failed: {path}");
}

/// Stitches two different-resolution clips plus a looped music bed and confirms
/// ffmpeg actually runs the graph and the output is a ~4s A/V MP4 (spec §4.3).
#[test]
fn stitches_two_clips_with_looped_music() {
    let dir = std::env::temp_dir();
    let a = dir.join("clipsnip_seq_a.mp4");
    let b = dir.join("clipsnip_seq_b.mp4");
    let music = dir.join("clipsnip_seq_music.m4a");
    let out = dir.join("clipsnip_seq_out.mp4");
    let (a_s, b_s, m_s, out_s) = (
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        music.to_str().unwrap(),
        out.to_str().unwrap(),
    );
    let _ = std::fs::remove_file(&out);

    gen_clip(a_s, "320x240", 440);
    gen_clip(b_s, "640x480", 660);
    // A 1s music bed so looping to the 4s sequence length is exercised.
    let mgen = Command::new("ffmpeg")
        .args(["-y", "-f", "lavfi", "-i", "sine=frequency=220:duration=1", "-c:a", "aac", m_s])
        .status()
        .expect("ffmpeg runs");
    assert!(mgen.success(), "music generation failed");

    let segments = vec![
        Segment { input: a_s.into(), in_secs: 0.0, out_secs: 2.0 },
        Segment { input: b_s.into(), in_secs: 0.0, out_secs: 2.0 },
    ];
    let overlay = MusicOverlay { path: m_s.into(), volume: 0.3, loop_to_length: true };
    let args = build_sequence_args(&segments, 640, 360, Some(&overlay), out_s);

    let run = Command::new("ffmpeg").args(args).status().expect("ffmpeg runs");
    assert!(run.success(), "sequence export failed");

    let bytes = std::fs::read(&out).expect("output exists");
    assert_eq!(&bytes[4..8], b"ftyp", "not an MP4");

    // ~4s (2 + 2), one video + one audio stream at the target height.
    let probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration:stream=codec_type,height",
            "-of", "default=noprint_wrappers=1",
            out_s,
        ])
        .output()
        .expect("ffprobe runs");
    let info = String::from_utf8_lossy(&probe.stdout);
    assert!(info.contains("codec_type=video"), "no video stream: {info}");
    assert!(info.contains("codec_type=audio"), "no audio stream: {info}");
    assert!(info.contains("height=360"), "wrong target height: {info}");
    let dur: f64 = info
        .lines()
        .find_map(|l| l.strip_prefix("duration="))
        .and_then(|d| d.parse().ok())
        .expect("duration present");
    assert!((dur - 4.0).abs() < 0.6, "stitched duration off: {dur}");

    for p in [&a, &b, &music, &out] {
        let _ = std::fs::remove_file(p);
    }
}
