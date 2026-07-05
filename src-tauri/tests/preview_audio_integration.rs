use std::process::Command;

use clipsnip_lib::media::{build_audio_chunk_args, build_video_only_args, patch_wav_sizes};

/// Generates the same kind of file `prepare_audio_tracks` produces: an AAC
/// m4a with a known sine tone.
fn generate_tone_m4a(path: &str, secs: u32) {
    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", &format!("sine=frequency=440:sample_rate=48000:duration={secs}"),
            "-c:a", "aac", "-b:a", "192k",
            "-movflags", "+faststart",
            path,
        ])
        .status()
        .expect("ffmpeg generate runs");
    assert!(gen.success(), "tone generation failed");
}

fn decode_chunk(src: &str, start: f64, dur: f64) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(build_audio_chunk_args(src, start, dur))
        .output()
        .expect("ffmpeg chunk decode runs");
    assert!(
        out.status.success(),
        "chunk decode failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// Extracts the PCM payload that follows the `data` chunk header.
fn wav_payload(wav: &[u8]) -> &[u8] {
    let mut off = 12;
    while off + 8 <= wav.len() {
        if &wav[off..off + 4] == b"data" {
            return &wav[off + 8..];
        }
        let size = u32::from_le_bytes(wav[off + 4..off + 8].try_into().unwrap()) as usize;
        off += 8 + size + (size & 1);
    }
    panic!("no data chunk in WAV output");
}

/// The chunked decode path must produce a well-formed WAV of the right length:
/// pipe output gets its placeholder sizes patched, and a mid-file chunk carries
/// the expected amount of PCM for its window.
#[test]
fn audio_chunk_decodes_patchable_wav_of_expected_duration() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_chunk_src.m4a");
    let src_s = src.to_str().unwrap();
    generate_tone_m4a(src_s, 8);

    let mut wav = decode_chunk(src_s, 2.0, 3.0);
    assert!(patch_wav_sizes(&mut wav), "output is not a WAV");

    let riff = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
    assert_eq!(riff, wav.len() - 8, "RIFF size not patched to real length");

    // 3s of mono s16 @48kHz = 288000 bytes; AAC framing may shift the edges by
    // a frame (1024 samples), so allow a small tolerance.
    let payload = wav_payload(&wav).len();
    let expected = 3 * 48000 * 2;
    assert!(
        (payload as i64 - expected as i64).unsigned_abs() < 8192,
        "unexpected PCM length: got {payload}, expected ~{expected}"
    );

    let _ = std::fs::remove_file(&src);
}

/// Two adjacent chunks must cover the same samples as one wide chunk — that is
/// what makes back-to-back scheduling on the AudioContext gapless. Alignment
/// must be sample-exact (equal lengths, no shift); amplitudes may differ by a
/// couple of s16 LSBs because the AAC decoder's float output converges over its
/// warm-up, which is far below audibility but breaks byte equality.
#[test]
fn adjacent_audio_chunks_join_gaplessly() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_gapless_src.m4a");
    let src_s = src.to_str().unwrap();
    generate_tone_m4a(src_s, 8);

    let a = decode_chunk(src_s, 1.0, 2.0);
    let b = decode_chunk(src_s, 3.0, 2.0);
    let whole = decode_chunk(src_s, 1.0, 4.0);

    let joined: Vec<u8> = [wav_payload(&a), wav_payload(&b)].concat();
    let reference = wav_payload(&whole);
    assert_eq!(
        joined.len(),
        reference.len(),
        "joined chunks and continuous decode diverge in sample count"
    );

    let mut max_delta = 0i32;
    for (j, r) in joined.chunks_exact(2).zip(reference.chunks_exact(2)) {
        let js = i16::from_le_bytes([j[0], j[1]]) as i32;
        let rs = i16::from_le_bytes([r[0], r[1]]) as i32;
        max_delta = max_delta.max((js - rs).abs());
    }
    // A misaligned seam on a 440Hz tone produces deltas in the thousands; the
    // decoder's rounding jitter stays within a couple of LSBs.
    assert!(max_delta <= 2, "chunk seam deviates audibly: max sample delta {max_delta}");

    let _ = std::fs::remove_file(&src);
}

/// The video-only remux must keep the video stream bit-identical in codec terms
/// (stream copy) while carrying zero audio streams — that absence is what stops
/// the WebView from registering an OS audio stream for the <video> element.
#[test]
fn video_only_remux_strips_all_audio_streams() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_vo_src.mp4");
    let out = dir.join("clipsnip_vo_out.mp4");
    let src_s = src.to_str().unwrap();
    let out_s = out.to_str().unwrap();
    let _ = std::fs::remove_file(&out);

    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=2:size=640x360:rate=30",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
            "-f", "lavfi", "-i", "sine=frequency=880:duration=2",
            "-map", "0:v", "-map", "1:a", "-map", "2:a",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            src_s,
        ])
        .status()
        .expect("ffmpeg generate runs");
    assert!(gen.success(), "source generation failed");

    let make = Command::new("ffmpeg")
        .args(build_video_only_args(src_s, out_s))
        .status()
        .expect("ffmpeg remux runs");
    assert!(make.success(), "video-only remux failed");

    let probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "stream=codec_type",
            "-of", "default=noprint_wrappers=1",
            out_s,
        ])
        .output()
        .expect("ffprobe runs");
    let info = String::from_utf8_lossy(&probe.stdout);
    assert!(info.contains("codec_type=video"), "video stream missing: {info}");
    assert!(!info.contains("codec_type=audio"), "audio stream survived the remux: {info}");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);
}
