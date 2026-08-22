//! Hardware smoke tests for GPU-accelerated export (2026-07-05 design).
//!
//! `#[ignore]`d because they need a working VA-API device (CI runners have none).
//! Run manually on a GPU machine with:
//!   cargo test --test vaapi_smoke -- --ignored

use std::process::Command;

use snippit_lib::export::{
    build_sequence_args, build_trim_compatible_args, ExportDevice, MusicOverlay, Segment,
    SequenceTarget, TrackConfig,
};

/// Stream 0 video, stream 1 silent audio, stream 2 a tone — the layout that
/// reproduced the "only the first track exports" bug.
fn gen_two_track_clip(path: &str) {
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=2:size=640x360:rate=30",
            "-f", "lavfi", "-t", "2", "-i", "anullsrc=r=44100:cl=stereo",
            "-f", "lavfi", "-i", "sine=frequency=880:duration=2",
            "-map", "0:v", "-map", "1:a", "-map", "2:a",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            path,
        ])
        .status()
        .expect("ffmpeg runs");
    assert!(ok.success(), "clip generation failed");
}

fn assert_h264_output(path: &str) {
    let probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "stream=codec_type,codec_name",
            "-of", "default=noprint_wrappers=1",
            path,
        ])
        .output()
        .expect("ffprobe runs");
    let info = String::from_utf8_lossy(&probe.stdout);
    assert!(info.contains("codec_name=h264"), "not h264: {info}");
    assert!(info.contains("codec_type=audio"), "no audio stream: {info}");
}

/// Compatible trim on VAAPI with a multi-track mix — exercises the simple `-vf`
/// hwupload graph coexisting with the audio-only `-filter_complex`.
#[test]
#[ignore]
fn vaapi_compatible_trim_with_amix_encodes() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_vaapi_trim_src.mp4");
    let out = dir.join("clipsnip_vaapi_trim_out.mp4");
    let (src_s, out_s) = (src.to_str().unwrap(), out.to_str().unwrap());
    gen_two_track_clip(src_s);
    let _ = std::fs::remove_file(&out);

    let tracks = vec![
        TrackConfig { stream_index: 1, enabled: true, volume: 0.8 },
        TrackConfig { stream_index: 2, enabled: true, volume: 1.0 },
    ];
    let args = build_trim_compatible_args(src_s, 0.0, 2.0, out_s, &tracks, &ExportDevice::Vaapi);
    let run = Command::new("ffmpeg").args(&args).output().expect("ffmpeg runs");
    assert!(
        run.status.success(),
        "vaapi compatible trim failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_h264_output(out_s);

    for p in [&src, &out] {
        let _ = std::fs::remove_file(p);
    }
}

/// Sequence on VAAPI with a size target (single-pass VBV), a multi-track segment,
/// an all-muted segment (anullsrc), and looped music — the full graph.
#[test]
#[ignore]
fn vaapi_sequence_with_target_size_encodes() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_vaapi_seq_src.mp4");
    let music = dir.join("clipsnip_vaapi_seq_music.m4a");
    let out = dir.join("clipsnip_vaapi_seq_out.mp4");
    let (src_s, m_s, out_s) = (
        src.to_str().unwrap(),
        music.to_str().unwrap(),
        out.to_str().unwrap(),
    );
    gen_two_track_clip(src_s);
    let _ = std::fs::remove_file(&out);
    let mgen = Command::new("ffmpeg")
        .args(["-y", "-f", "lavfi", "-i", "sine=frequency=220:duration=1", "-c:a", "aac", m_s])
        .status()
        .expect("ffmpeg runs");
    assert!(mgen.success(), "music generation failed");

    let segments = vec![
        Segment {
            input: src_s.into(),
            in_secs: 0.0,
            out_secs: 2.0,
            tracks: vec![
                TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
                TrackConfig { stream_index: 2, enabled: true, volume: 1.0 },
            ],
        },
        Segment {
            input: src_s.into(),
            in_secs: 0.0,
            out_secs: 1.0,
            tracks: vec![
                TrackConfig { stream_index: 1, enabled: false, volume: 1.0 },
                TrackConfig { stream_index: 2, enabled: false, volume: 1.0 },
            ],
        },
    ];
    let overlay = MusicOverlay { path: m_s.into(), volume: 0.4, loop_to_length: true };
    let target = SequenceTarget {
        video_kbps: 800,
        audio_kbps: 128,
        pass: 2, // GPU: single pass
        passlog: dir.join("clipsnip_vaapi_seq_passlog").to_string_lossy().into_owned(),
    };
    let args = build_sequence_args(
        &segments, 640, 360, Some(&overlay), out_s, &ExportDevice::Vaapi, Some(&target),
    );
    let run = Command::new("ffmpeg").args(&args).output().expect("ffmpeg runs");
    assert!(
        run.status.success(),
        "vaapi sequence export failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_h264_output(out_s);

    for p in [&src, &music, &out] {
        let _ = std::fs::remove_file(p);
    }
}
