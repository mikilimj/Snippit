use std::process::Command;

use snippit_lib::export::{build_sequence_args, ExportDevice, MusicOverlay, Segment, TrackConfig};

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
        Segment { input: a_s.into(), in_secs: 0.0, out_secs: 2.0, tracks: vec![] },
        Segment { input: b_s.into(), in_secs: 0.0, out_secs: 2.0, tracks: vec![] },
    ];
    let overlay = MusicOverlay { path: m_s.into(), volume: 0.3, loop_to_length: true };
    let args =
        build_sequence_args(&segments, 640, 360, Some(&overlay), out_s, &ExportDevice::Cpu, None);

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

/// Regression test for the "only the first audio track survives sequence export"
/// bug: a clip whose FIRST audio track is silence and whose SECOND carries a tone
/// must still produce an audible sequence when both tracks are enabled. The old
/// `[i:a]` mapping picked only the silent first track.
#[test]
fn sequence_mixes_all_enabled_audio_tracks_of_a_segment() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_seq_multitrack_src.mkv");
    let out = dir.join("clipsnip_seq_multitrack_out.mp4");
    let (src_s, out_s) = (src.to_str().unwrap(), out.to_str().unwrap());
    let _ = std::fs::remove_file(&out);

    // Stream 0 video, stream 1 silent audio, stream 2 a 880 Hz tone.
    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=30",
            "-f", "lavfi", "-t", "2", "-i", "anullsrc=r=44100:cl=stereo",
            "-f", "lavfi", "-i", "sine=frequency=880:duration=2",
            "-map", "0:v", "-map", "1:a", "-map", "2:a",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            src_s,
        ])
        .status()
        .expect("ffmpeg runs");
    assert!(gen.success(), "multi-track clip generation failed");

    let segments = vec![Segment {
        input: src_s.into(),
        in_secs: 0.0,
        out_secs: 2.0,
        tracks: vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 1.0 },
        ],
    }];
    let args = build_sequence_args(&segments, 320, 240, None, out_s, &ExportDevice::Cpu, None);
    let run = Command::new("ffmpeg").args(args).status().expect("ffmpeg runs");
    assert!(run.success(), "sequence export failed");

    // Output must have exactly one audio stream, and it must carry the tone —
    // the old mapping would have exported the silent track instead.
    let probe = Command::new("ffmpeg")
        .args(["-i", out_s, "-map", "0:a:0", "-af", "volumedetect", "-f", "null", "-"])
        .output()
        .expect("ffmpeg volumedetect runs");
    let stderr = String::from_utf8_lossy(&probe.stderr);
    let mean_db: f64 = stderr
        .lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .and_then(|v| v.trim().trim_end_matches(" dB").parse().ok())
        .expect("mean_volume present");
    assert!(
        mean_db > -50.0,
        "sequence audio is silent ({mean_db} dB) — second track was dropped"
    );

    for p in [&src, &out] {
        let _ = std::fs::remove_file(p);
    }
}
