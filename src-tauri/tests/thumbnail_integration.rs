use std::process::Command;

use clipsnip_lib::media::build_thumbnail_args;

#[test]
fn generates_a_real_jpeg_thumbnail_from_a_clip() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_thumb_src.mp4");
    let thumb = dir.join("clipsnip_thumb_out.jpg");
    let src_s = src.to_str().unwrap();
    let thumb_s = thumb.to_str().unwrap();
    let _ = std::fs::remove_file(&thumb);

    // Generate a 3s 320x240 clip.
    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=3:size=320x240:rate=30",
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
            src_s,
        ])
        .status()
        .expect("ffmpeg generate runs");
    assert!(gen.success(), "clip generation failed");

    // Build the thumbnail at t=1.0s, width 160.
    let make = Command::new("ffmpeg")
        .args(build_thumbnail_args(src_s, thumb_s, 1.0, 160))
        .status()
        .expect("ffmpeg thumbnail runs");
    assert!(make.success(), "thumbnail generation failed");

    let bytes = std::fs::read(&thumb).expect("thumbnail exists");
    assert!(!bytes.is_empty(), "thumbnail is empty");
    // JPEG SOI marker.
    assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "not a JPEG");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&thumb);
}
