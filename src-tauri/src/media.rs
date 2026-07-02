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
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    mtime.hash(&mut h);
    cache_dir.join(format!("{:x}.jpg", h.finish()))
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
}
