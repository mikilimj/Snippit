//! Media helpers: thumbnail (and, later, proxy) generation. The FFmpeg argument
//! builders are pure so they can be asserted in unit tests; the actual sidecar
//! invocation lives in the async orchestration that has the Tauri `AppHandle`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// FFmpeg arguments to grab a single frame at `at_secs` as a width-`width` JPEG
/// (height auto to preserve aspect). `-ss` precedes `-i` for a fast keyframe seek.
pub fn build_thumbnail_args(source: &str, out_path: &str, at_secs: f64, width: u32) -> Vec<String> {
    vec![
        "-y".into(),
        "-ss".into(),
        format!("{at_secs}"),
        "-i".into(),
        source.into(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        format!("scale={width}:-1"),
        "-q:v".into(),
        "4".into(),
        out_path.into(),
    ]
}

/// Deterministic on-disk cache location for a clip's thumbnail, keyed by source
/// path + mtime so a re-encoded file (new mtime) gets a fresh thumbnail while an
/// unchanged file reuses the cached one (spec §4.4 / §9 cache philosophy).
pub fn thumbnail_path(cache_dir: &Path, source: &str, mtime: i64) -> PathBuf {
    cache_dir.join(format!("{:x}.jpg", hash_key(source, mtime)))
}

fn hash_key(source: &str, mtime: i64) -> u64 {
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    mtime.hash(&mut h);
    h.finish()
}

/// FFmpeg arguments to transcode a clip the WebView can't decode natively into a
/// lightweight 720p H.264/AAC MP4 proxy for smooth preview scrubbing (spec §4.4).
/// The proxy is preview-only — export always reads the untouched original.
pub fn build_proxy_args(source: &str, out_path: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-i".into(),
        source.into(),
        // Cap height at 720 without upscaling; width auto, kept even for H.264.
        "-vf".into(),
        "scale=-2:'min(720,ih)'".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "23".into(),
        "-c:a".into(),
        "aac".into(),
        "-movflags".into(),
        "+faststart".into(),
        out_path.into(),
    ]
}

/// Deterministic on-disk cache location for a clip's preview proxy, keyed like
/// the thumbnail (source path + mtime). The `.mp4` extension keeps it distinct
/// from the same clip's `.jpg` thumbnail in the shared cache directory.
pub fn proxy_path(cache_dir: &Path, source: &str, mtime: i64) -> PathBuf {
    cache_dir.join(format!("{:x}.mp4", hash_key(source, mtime)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn thumbnail_args_seek_before_input_and_single_frame() {
        let args = build_thumbnail_args("/v/a.mp4", "/c/a.jpg", 2.5, 320);
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        let i = args.iter().position(|a| a == "-i").unwrap();
        // -ss before -i => fast (keyframe) seek without decoding from the start.
        assert!(ss < i);
        assert_eq!(args[ss + 1], "2.5");
        assert_eq!(args[i + 1], "/v/a.mp4");
        assert!(args.windows(2).any(|w| w[0] == "-frames:v" && w[1] == "1"));
        assert!(args.iter().any(|a| a.contains("scale=320")));
        assert_eq!(args.last().unwrap(), "/c/a.jpg");
        // Overwrite so a stale/partial thumbnail is always replaced.
        assert!(args.iter().any(|a| a == "-y"));
    }

    #[test]
    fn thumbnail_path_is_deterministic_by_source_and_mtime() {
        let dir = Path::new("/cache");
        let p1 = thumbnail_path(dir, "/v/a.mp4", 111);
        let p2 = thumbnail_path(dir, "/v/a.mp4", 111);
        let p3 = thumbnail_path(dir, "/v/a.mp4", 222);
        assert_eq!(p1, p2); // same source + mtime => same cache key
        assert_ne!(p1, p3); // a re-encode (new mtime) gets a fresh thumbnail
        assert!(p1.starts_with(dir));
        assert_eq!(p1.extension().and_then(|e| e.to_str()), Some("jpg"));
    }

    #[test]
    fn thumbnail_path_differs_per_source() {
        let dir = Path::new("/cache");
        assert_ne!(
            thumbnail_path(dir, "/v/a.mp4", 111),
            thumbnail_path(dir, "/v/b.mp4", 111)
        );
    }

    #[test]
    fn proxy_args_transcode_to_720p_h264_mp4() {
        let args = build_proxy_args("/v/a.mkv", "/c/a.mp4");
        let i = args.iter().position(|a| a == "-i").unwrap();
        assert_eq!(args[i + 1], "/v/a.mkv");
        // Downscale to 720 lines (width auto, kept even for H.264).
        assert!(args.iter().any(|a| a.contains("scale=") && a.contains("720")));
        // H.264 video + AAC audio so the WebView can always decode it.
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
        assert!(args.windows(2).any(|w| w[0] == "-c:a" && w[1] == "aac"));
        // faststart => progressive playback while still generating/serving.
        assert!(args.windows(2).any(|w| w[0] == "-movflags" && w[1] == "+faststart"));
        assert!(args.iter().any(|a| a == "-y")); // overwrite partial files
        assert_eq!(args.last().unwrap(), "/c/a.mp4");
    }

    #[test]
    fn proxy_path_is_deterministic_by_source_and_mtime() {
        let dir = Path::new("/cache");
        let p1 = proxy_path(dir, "/v/a.mkv", 111);
        let p2 = proxy_path(dir, "/v/a.mkv", 111);
        let p3 = proxy_path(dir, "/v/a.mkv", 222);
        assert_eq!(p1, p2); // same source + mtime => reuse the cached proxy
        assert_ne!(p1, p3); // a re-encode (new mtime) gets a fresh proxy
        assert!(p1.starts_with(dir));
        assert_eq!(p1.extension().and_then(|e| e.to_str()), Some("mp4"));
    }

    #[test]
    fn proxy_path_differs_from_thumbnail_for_same_clip() {
        // Proxy (.mp4) and thumbnail (.jpg) must never collide in the cache dir.
        let dir = Path::new("/cache");
        assert_ne!(proxy_path(dir, "/v/a.mkv", 111), thumbnail_path(dir, "/v/a.mkv", 111));
    }
}
