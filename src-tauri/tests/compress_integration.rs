use std::process::Command;

use clipsnip_lib::export::{build_compress_pass_args, compute_target_bitrates, TrackConfig};

#[test]
fn two_pass_compress_lands_at_or_under_the_target_size() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_compress_src.mp4");
    let dst = dir.join("clipsnip_compress_dst.mp4");
    let passlog = dir.join("clipsnip_compress_passlog");
    let src_s = src.to_str().unwrap();
    let dst_s = dst.to_str().unwrap();
    let passlog_s = passlog.to_str().unwrap();

    // 10s test clip with one audio track.
    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=10:size=640x360:rate=30",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=10",
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
            "-c:a", "aac",
            src_s,
        ])
        .status()
        .expect("ffmpeg generate runs");
    assert!(gen.success(), "clip generation failed");

    let target_mb = 1.0;
    let (video_kbps, audio_kbps) = compute_target_bitrates(target_mb, 8.0).unwrap();
    let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];

    for pass in [1u8, 2u8] {
        let args = build_compress_pass_args(
            src_s, 1.0, 9.0, dst_s, &tracks, video_kbps, audio_kbps, pass, passlog_s,
        );
        let status = Command::new("ffmpeg").args(&args).status().expect("ffmpeg pass runs");
        assert!(status.success(), "pass {pass} failed");
    }

    let size = std::fs::metadata(&dst).expect("output exists").len() as f64;
    let target_bytes = target_mb * 1024.0 * 1024.0;
    // ABR rate control isn't exact; the 0.97 safety factor plus a 10% tolerance
    // is the "within tolerance of the requested target" bar from the design spec.
    assert!(
        size <= target_bytes * 1.10,
        "output {size} bytes exceeds {target_bytes} target by more than 10%"
    );
    assert!(size > 0.0, "output is empty");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
    let _ = std::fs::remove_file(dir.join("clipsnip_compress_passlog-0.log"));
    let _ = std::fs::remove_file(dir.join("clipsnip_compress_passlog-0.log.mbtree"));
}
