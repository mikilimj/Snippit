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

/// Decoder warm-up fed to ffmpeg before each audio chunk's real start. Input-side
/// `-ss` alone lands on the AAC frame grid (~21ms granularity) with a cold MDCT
/// overlap, so seams between chunks would jump; seeking one second early and
/// trimming on the output side decodes past the warm-up and cuts sample-exact.
pub const AUDIO_CHUNK_PREROLL: f64 = 1.0;

/// FFmpeg arguments to decode one time window of an extracted preview audio
/// track to a 16-bit PCM WAV file. The frontend schedules these chunks on a
/// single shared AudioContext instead of per-track <audio> elements, so the app
/// registers exactly one audio stream with the OS mixer. The input seek is a
/// fast packet-level jump to `start - PREROLL`; the output-side `-ss`/`-t` then
/// trim to the exact sample window, which keeps adjacent chunks gapless.
///
/// The WAV goes to a temp file, never to stdout: the shell plugin's `output()`
/// collects stdout line by line and re-joins with `\n`, which mangles binary
/// PCM (every 0x0A byte is a "line break") into loud static.
pub fn build_audio_chunk_args(track_path: &str, start: f64, dur: f64, out_path: &str) -> Vec<String> {
    let pre = (start - AUDIO_CHUNK_PREROLL).max(0.0);
    let out_ss = start - pre;
    vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-ss".into(),
        format!("{pre}"),
        "-i".into(),
        track_path.into(),
        "-ss".into(),
        format!("{out_ss}"),
        "-t".into(),
        format!("{dur}"),
        "-acodec".into(),
        "pcm_s16le".into(),
        "-f".into(),
        "wav".into(),
        out_path.into(),
    ]
}

/// FFmpeg writes placeholder (0xFFFFFFFF) RIFF and `data` chunk sizes when the
/// WAV goes to a pipe, because it can't seek back to fix them. Some decoders
/// (WebKit's decodeAudioData among them) reject that, so rewrite both sizes
/// from the actual byte count. Returns false if the buffer isn't a WAV.
pub fn patch_wav_sizes(bytes: &mut [u8]) -> bool {
    let len = bytes.len();
    if len < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return false;
    }
    let riff_size = (len as u64 - 8).min(u32::MAX as u64) as u32;
    bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());

    // Walk the chunk list to find `data`; ffmpeg may emit LIST/fact chunks first.
    let mut off = 12;
    while off + 8 <= len {
        let id = &bytes[off..off + 4];
        if id == b"data" {
            let data_size = (len as u64 - (off as u64 + 8)).min(u32::MAX as u64) as u32;
            bytes[off + 4..off + 8].copy_from_slice(&data_size.to_le_bytes());
            return true;
        }
        let size = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()) as usize;
        // Chunks are word-aligned: odd sizes carry one pad byte.
        off += 8 + size + (size & 1);
    }
    false
}

/// FFmpeg arguments to remux a preview source into a video-only copy (all video
/// streams, no audio/subtitles, no re-encode). Served to the <video> element
/// once Web Audio owns playback: with no audio track in the file, the WebView
/// never builds an audio pipeline for it, so the muted-but-visible OS audio
/// stream the element would otherwise register disappears.
pub fn build_video_only_args(source: &str, out_path: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-i".into(),
        source.into(),
        "-map".into(),
        "0:v".into(),
        "-c".into(),
        "copy".into(),
    ];
    let ext = out_path.rsplit('.').next().unwrap_or("").to_lowercase();
    if matches!(ext.as_str(), "mp4" | "m4v" | "mov") {
        // moov atom up front so playback starts without a seek to EOF.
        args.push("-movflags".into());
        args.push("+faststart".into());
    }
    args.push(out_path.into());
    args
}

/// Cache location for a clip's video-only preview remux. Keeps the source's
/// container (same demuxer that already played) and a `-vo` marker so it can
/// never collide with the same clip's proxy (`{hash}.mp4`) in the cache dir.
pub fn video_only_path(cache_dir: &Path, source: &str, mtime: i64) -> PathBuf {
    let ext = match source.rsplit('.').next() {
        Some(e) if !e.contains(['/', '\\']) && !e.is_empty() => e.to_lowercase(),
        _ => "mp4".to_string(),
    };
    cache_dir.join(format!("{:x}-vo.{ext}", hash_key(source, mtime)))
}

/// Target size for a percent-of-original re-encode, in MB (fed to
/// export::compute_target_bitrates).
pub fn percent_target_mb(size_bytes: u64, percent: u8) -> f64 {
    (size_bytes as f64 / (1024.0 * 1024.0)) * (percent as f64 / 100.0)
}

/// Whole-file single-pass re-encode to a bitrate budget: video per device
/// (VBV-capped), all audio streams kept and re-encoded to AAC. Used for the
/// upload-size and local-shrink paths; unlike exports there is no trim window
/// and no track mixing — streams are preserved 1:1.
pub fn build_size_reencode_args(
    source: &str,
    output: &str,
    video_kbps: u32,
    audio_kbps: u32,
    device: &crate::export::ExportDevice,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-y".into()];
    args.extend(device.global_args());
    args.extend(device.input_args());
    args.extend(["-i".into(), source.into()]);
    args.extend(["-map".into(), "0:v:0".into(), "-map".into(), "0:a?".into()]);
    if let Some(suffix) = device.filter_suffix() {
        args.extend(["-vf".to_string(), suffix.to_string()]);
    }
    args.extend(device.video_bitrate_args(video_kbps));
    args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), format!("{audio_kbps}k")]);
    args.extend([
        "-movflags".to_string(),
        "+faststart".to_string(),
    ]);
    args.push(output.into());
    args
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
    fn audio_chunk_args_preroll_input_seek_and_exact_output_trim() {
        let args = build_audio_chunk_args("/tmp/track.m4a", 40.0, 20.0, "/tmp/chunk.wav");
        let i = args.iter().position(|a| a == "-i").unwrap();
        // Fast input seek to start-preroll, then exact output-side -ss/-t trim.
        let in_ss = args.iter().position(|a| a == "-ss").unwrap();
        assert!(in_ss < i);
        assert_eq!(args[in_ss + 1], "39");
        assert_eq!(args[i + 1], "/tmp/track.m4a");
        let out_ss = i + args[i..].iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[out_ss + 1], "1");
        let t = args.iter().position(|a| a == "-t").unwrap();
        assert!(t > i);
        assert_eq!(args[t + 1], "20");
        assert!(args.windows(2).any(|w| w[0] == "-acodec" && w[1] == "pcm_s16le"));
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "wav"));
        // A temp file, NEVER stdout — the shell plugin line-splits stdout and
        // corrupts binary PCM.
        assert_eq!(args.last().unwrap(), "/tmp/chunk.wav");
        assert!(args.iter().any(|a| a == "-y")); // overwrite a stale temp file
    }

    #[test]
    fn audio_chunk_args_clamp_preroll_at_file_start() {
        // A chunk starting inside the preroll window seeks to 0 and trims the
        // real start on the output side.
        let args = build_audio_chunk_args("/tmp/track.m4a", 0.4, 20.0, "/tmp/chunk.wav");
        let i = args.iter().position(|a| a == "-i").unwrap();
        let in_ss = args.iter().position(|a| a == "-ss").unwrap();
        assert!(in_ss < i);
        assert_eq!(args[in_ss + 1], "0");
        let out_ss = i + args[i..].iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[out_ss + 1], "0.4");
    }

    /// Minimal canonical WAV: RIFF header + fmt + data with `n` payload bytes,
    /// sizes deliberately set to ffmpeg's pipe placeholder 0xFFFFFFFF.
    fn wav_with_placeholder_sizes(n: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&u32::MAX.to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&[0u8; 16]);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&u32::MAX.to_le_bytes());
        v.extend_from_slice(&vec![0u8; n]);
        v
    }

    #[test]
    fn patch_wav_sizes_rewrites_riff_and_data_sizes() {
        let mut wav = wav_with_placeholder_sizes(1000);
        let total = wav.len();
        assert!(patch_wav_sizes(&mut wav));
        let riff = u32::from_le_bytes(wav[4..8].try_into().unwrap());
        assert_eq!(riff as usize, total - 8);
        let data_off = 36; // 12 RIFF/WAVE + 24 fmt chunk
        assert_eq!(&wav[data_off..data_off + 4], b"data");
        let data = u32::from_le_bytes(wav[data_off + 4..data_off + 8].try_into().unwrap());
        assert_eq!(data as usize, 1000);
    }

    #[test]
    fn patch_wav_sizes_rejects_non_wav() {
        let mut junk = vec![0u8; 100];
        assert!(!patch_wav_sizes(&mut junk));
        let mut short = b"RIFF".to_vec();
        assert!(!patch_wav_sizes(&mut short));
    }

    #[test]
    fn video_only_args_copy_video_streams_only() {
        let args = build_video_only_args("/v/a.mkv", "/c/a-vo.mkv");
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "0:v"));
        assert!(args.windows(2).any(|w| w[0] == "-c" && w[1] == "copy"));
        // mkv output: no mp4-only faststart flag
        assert!(!args.iter().any(|a| a == "-movflags"));
        assert!(args.iter().any(|a| a == "-y"));
        assert_eq!(args.last().unwrap(), "/c/a-vo.mkv");
    }

    #[test]
    fn video_only_args_mp4_gets_faststart() {
        let args = build_video_only_args("/v/a.mp4", "/c/a-vo.mp4");
        assert!(args.windows(2).any(|w| w[0] == "-movflags" && w[1] == "+faststart"));
    }

    #[test]
    fn video_only_path_keeps_container_and_never_collides_with_proxy() {
        let dir = Path::new("/cache");
        let p = video_only_path(dir, "/v/a.MKV", 111);
        assert_eq!(p.extension().and_then(|e| e.to_str()), Some("mkv"));
        // same clip, same cache dir: proxy and video-only remux are distinct files
        assert_ne!(video_only_path(dir, "/v/a.mp4", 111), proxy_path(dir, "/v/a.mp4", 111));
        // extensionless sources fall back to mp4 instead of an empty extension
        let none = video_only_path(dir, "/v/noext", 111);
        assert_eq!(none.extension().and_then(|e| e.to_str()), Some("mp4"));
    }

    #[test]
    fn proxy_path_differs_from_thumbnail_for_same_clip() {
        // Proxy (.mp4) and thumbnail (.jpg) must never collide in the cache dir.
        let dir = Path::new("/cache");
        assert_ne!(proxy_path(dir, "/v/a.mkv", 111), thumbnail_path(dir, "/v/a.mkv", 111));
    }

    #[test]
    fn percent_target_mb_scales_bytes() {
        // 100 MB file at 50% -> 50.0 MB target
        assert!((percent_target_mb(100 * 1024 * 1024, 50) - 50.0).abs() < 1e-9);
        assert!((percent_target_mb(10 * 1024 * 1024, 25) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn build_size_reencode_args_with_vaapi_includes_hwupload_filter() {
        let args = build_size_reencode_args(
            "/c/in.mp4", "/c/out.mp4", 2500, 128, &crate::export::ExportDevice::Vaapi);
        let joined = args.join(" ");
        // VAAPI requires hwupload filter to convert software frames to hardware.
        assert!(joined.contains("-vf"), "Missing -vf flag for VAAPI filter");
        assert!(joined.contains("hwupload"), "Missing hwupload filter for VAAPI");
        assert!(joined.contains("format=nv12"), "Missing format=nv12 for VAAPI");
    }

    #[test]
    fn build_size_reencode_args_includes_faststart() {
        let args = build_size_reencode_args(
            "/c/in.mp4", "/c/out.mp4", 2500, 128, &crate::export::ExportDevice::Cpu);
        let joined = args.join(" ");
        // faststart puts the moov atom at the start for streaming/upload.
        assert!(joined.contains("-movflags"), "Missing -movflags flag");
        assert!(joined.contains("+faststart"), "Missing +faststart");
        assert!(joined.ends_with("/c/out.mp4"));
    }

    #[test]
    fn build_size_reencode_args_maps_all_streams_and_sets_bitrates() {
        let args = build_size_reencode_args(
            "/c/in.mp4", "/c/out.mp4", 2500, 128, &crate::export::ExportDevice::Cpu);
        let joined = args.join(" ");
        assert!(joined.contains("-i /c/in.mp4"));
        assert!(joined.contains("-map 0:v:0"));
        assert!(joined.contains("-map 0:a?"));
        assert!(joined.contains("-b:a 128k"));
        assert!(joined.contains("2500k"));
        assert!(joined.ends_with("/c/out.mp4"));
        assert!(args.contains(&"-y".to_string()));
    }
}
