use std::process::Command;

use snippit_lib::media::build_proxy_args;

/// Builds a real proxy from a 1080p source and verifies ffprobe sees a 720p
/// H.264 stream — the format the WebView can always decode (spec §4.4).
#[test]
fn builds_a_playable_720p_h264_proxy() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_proxy_src.mp4");
    let out = dir.join("clipsnip_proxy_out.mp4");
    let src_s = src.to_str().unwrap();
    let out_s = out.to_str().unwrap();
    let _ = std::fs::remove_file(&out);

    // A 2s 1920x1080 source to prove the proxy downscales.
    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=2:size=1920x1080:rate=30",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            src_s,
        ])
        .status()
        .expect("ffmpeg generate runs");
    assert!(gen.success(), "source generation failed");

    let make = Command::new("ffmpeg")
        .args(build_proxy_args(src_s, out_s))
        .status()
        .expect("ffmpeg proxy runs");
    assert!(make.success(), "proxy generation failed");

    // MP4 files carry an "ftyp" box at offset 4.
    let bytes = std::fs::read(&out).expect("proxy exists");
    assert!(bytes.len() > 8, "proxy is too small");
    assert_eq!(&bytes[4..8], b"ftyp", "not an MP4 container");

    // Confirm the encoded stream is 720p H.264.
    let probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=codec_name,height",
            "-of", "default=noprint_wrappers=1",
            out_s,
        ])
        .output()
        .expect("ffprobe runs");
    let info = String::from_utf8_lossy(&probe.stdout);
    assert!(info.contains("codec_name=h264"), "proxy is not H.264: {info}");
    assert!(info.contains("height=720"), "proxy is not 720p: {info}");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);
}
