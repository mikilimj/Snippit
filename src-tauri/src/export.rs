use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackConfig {
    pub stream_index: u32,
    pub enabled: bool,
    pub volume: f64,
}

/// One clip in a stitched sequence: a source file and the in/out span to take
/// from it (spec §4.3). Straight cuts only — no transitions in v1.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub input: String,
    pub in_secs: f64,
    pub out_secs: f64,
    /// Per-track audio config for this clip, snapshotted from the preview when the
    /// segment was added. Empty = no config (legacy callers): map the default audio
    /// stream. Configured-but-all-muted = silence.
    #[serde(default)]
    pub tracks: Vec<TrackConfig>,
}

/// A music file overlaid across the whole sequence at `volume`, optionally looped
/// to the sequence length (spec §4.3).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicOverlay {
    pub path: String,
    pub volume: f64,
    pub loop_to_length: bool,
}

/// The device used for export re-encodes. `Cpu` is libx264; the rest are ffmpeg
/// hardware H.264 encoders. The sidecar compiles all of them in regardless of the
/// actual hardware, so callers must functionally probe a device before offering it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportDevice {
    Cpu,
    Vaapi,
    Nvenc,
    Qsv,
    Amf,
    VideoToolbox,
}

impl ExportDevice {
    /// Unknown/missing ids fall back to CPU so a stale setting can never break exports.
    pub fn from_id(id: &str) -> ExportDevice {
        match id {
            "vaapi" => Self::Vaapi,
            "nvenc" => Self::Nvenc,
            "qsv" => Self::Qsv,
            "amf" => Self::Amf,
            "videotoolbox" => Self::VideoToolbox,
            _ => Self::Cpu,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Vaapi => "vaapi",
            Self::Nvenc => "nvenc",
            Self::Qsv => "qsv",
            Self::Amf => "amf",
            Self::VideoToolbox => "videotoolbox",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Cpu => "CPU (software x264)",
            Self::Vaapi => "GPU — VA-API (AMD/Intel)",
            Self::Nvenc => "GPU — NVENC (NVIDIA)",
            Self::Qsv => "GPU — Quick Sync (Intel)",
            Self::Amf => "GPU — AMF (AMD)",
            Self::VideoToolbox => "GPU — VideoToolbox (Apple)",
        }
    }

    pub fn encoder(&self) -> &'static str {
        match self {
            Self::Cpu => "libx264",
            Self::Vaapi => "h264_vaapi",
            Self::Nvenc => "h264_nvenc",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::VideoToolbox => "h264_videotoolbox",
        }
    }

    /// Global options, placed before the first input. VAAPI needs an explicit
    /// hardware device shared by the `hwupload` filter and the encoder.
    pub fn global_args(&self) -> Vec<String> {
        match self {
            Self::Vaapi => ["-init_hw_device", "vaapi=hw", "-filter_hw_device", "hw"]
                .map(String::from)
                .to_vec(),
            _ => Vec::new(),
        }
    }

    /// Hardware *decode* flags, placed before each `-i`. Decoded frames stay in
    /// system memory so the software filters (scale/pad/concat) keep working, and
    /// ffmpeg falls back to software decoding per stream for codecs the hardware
    /// can't handle. QSV stays encode-only — its decode path can't fall back cleanly.
    pub fn input_args(&self) -> Vec<String> {
        match self {
            Self::Vaapi => ["-hwaccel", "vaapi"].map(String::from).to_vec(),
            Self::Nvenc => ["-hwaccel", "cuda"].map(String::from).to_vec(),
            Self::Amf => ["-hwaccel", "d3d11va"].map(String::from).to_vec(),
            Self::VideoToolbox => ["-hwaccel", "videotoolbox"].map(String::from).to_vec(),
            Self::Cpu | Self::Qsv => Vec::new(),
        }
    }

    /// Quality-mode video encode args — the Compatible / sequence default,
    /// replacing the old hardcoded `libx264 -preset veryfast -crf 20`.
    pub fn video_quality_args(&self) -> Vec<String> {
        let flags: &[&str] = match self {
            Self::Cpu => &["-preset", "veryfast", "-crf", "20"],
            Self::Vaapi => &["-qp", "22"],
            Self::Nvenc => &["-preset", "p5", "-cq", "23"],
            Self::Qsv => &["-global_quality", "23"],
            Self::Amf => &["-quality", "balanced", "-rc", "cqp", "-qp_i", "22", "-qp_p", "24"],
            Self::VideoToolbox => &["-q:v", "60"],
        };
        let mut args = vec!["-c:v".to_string(), self.encoder().to_string()];
        args.extend(flags.iter().map(|s| s.to_string()));
        args
    }

    /// Bitrate-mode video encode args (target-size exports). CPU callers add the
    /// two-pass flags on top; hardware encoders are single-pass, so they get a VBV
    /// cap (`-maxrate`/`-bufsize`) to keep the size near the target instead.
    pub fn video_bitrate_args(&self, video_kbps: u32) -> Vec<String> {
        let mut args = vec![
            "-c:v".to_string(),
            self.encoder().to_string(),
            "-b:v".to_string(),
            format!("{video_kbps}k"),
        ];
        match self {
            Self::Cpu => args.extend(["-preset".to_string(), "medium".to_string()]),
            _ => {
                if matches!(self, Self::Nvenc) {
                    args.extend(["-preset".to_string(), "p5".to_string()]);
                }
                args.extend([
                    "-maxrate".to_string(),
                    format!("{video_kbps}k"),
                    "-bufsize".to_string(),
                    format!("{}k", video_kbps * 2),
                ]);
            }
        }
        args
    }

    /// Filter-chain tail required to feed the encoder. VAAPI only accepts hardware
    /// frames, so software-filtered output must be uploaded as the last step.
    pub fn filter_suffix(&self) -> Option<&'static str> {
        match self {
            Self::Vaapi => Some("format=nv12,hwupload"),
            _ => None,
        }
    }

    /// Only x264 supports a real two-pass encode; hardware rate control is single-pass.
    pub fn supports_two_pass(&self) -> bool {
        matches!(self, Self::Cpu)
    }
}

/// Build ffmpeg args for a trim export with per-track audio control.
///
/// Fast path (stream copy): exactly one enabled track, at volume ≈ 1.0.
/// Mix path (re-encode audio): two or more enabled tracks (always mixed into a single
/// output stream, even at unity volume — a multi-stream container would leave every
/// player but the first track silent) or a single track that deviates from unity.
pub fn build_trim_args(
    input: &str,
    in_secs: f64,
    out_secs: f64,
    output: &str,
    tracks: &[TrackConfig],
) -> Vec<String> {
    let duration = out_secs - in_secs;
    let mut args = vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
    ];

    let enabled: Vec<&TrackConfig> = tracks.iter().filter(|t| t.enabled).collect();

    if enabled.is_empty() {
        // No explicit config or all muted → copy all streams as before
        args.extend(["-c".to_string(), "copy".to_string(), output.to_string()]);
        return args;
    }

    let single_track_unity =
        enabled.len() == 1 && (enabled[0].volume - 1.0).abs() < 0.001;

    if single_track_unity {
        // Fast path: only one audio stream in the output, so stream-copy is safe —
        // there's nothing to mix and no other track for a player to ignore.
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
        args.extend(["-map".to_string(), format!("0:{}", enabled[0].stream_index)]);
        args.extend(["-c".to_string(), "copy".to_string(), output.to_string()]);
        return args;
    }

    // Mix path: volume filter(s) + amix, re-encode audio
    let filter = amix_filter(&enabled);
    args.extend([
        "-filter_complex".to_string(),
        filter,
        "-map".to_string(),
        "0:v:0".to_string(),
        "-map".to_string(),
        "[aout]".to_string(),
        "-c:v".to_string(),
        "copy".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        output.to_string(),
    ]);
    args
}

fn amix_filter(enabled: &[&TrackConfig]) -> String {
    if enabled.len() == 1 {
        return format!("[0:{}]volume={:.3}[aout]", enabled[0].stream_index, enabled[0].volume);
    }
    let mut parts: Vec<String> = enabled
        .iter()
        .enumerate()
        .map(|(i, t)| format!("[0:{}]volume={:.3}[a{i}]", t.stream_index, t.volume))
        .collect();
    let labels: String = (0..enabled.len()).map(|i| format!("[a{i}]")).collect();
    parts.push(format!(
        "{}amix=inputs={}:duration=first:normalize=0[aout]",
        labels,
        enabled.len()
    ));
    parts.join(";")
}

/// Legacy wrapper kept for tests. Equivalent to build_trim_args with no track overrides
/// (copies all streams unchanged).
pub fn build_trim_copy_args(input: &str, in_secs: f64, out_secs: f64, output: &str) -> Vec<String> {
    build_trim_args(input, in_secs, out_secs, output, &[])
}

/// Build ffmpeg args for a **Compatible MP4** trim export (spec §4.5): the video
/// is always re-encoded to H.264 (and audio to AAC) for universal playback and
/// frame-accurate cut points, rather than stream-copied at keyframes. Audio track
/// mapping/mixing follows the same rules as `build_trim_args`.
pub fn build_trim_compatible_args(
    input: &str,
    in_secs: f64,
    out_secs: f64,
    output: &str,
    tracks: &[TrackConfig],
    device: &ExportDevice,
) -> Vec<String> {
    let duration = out_secs - in_secs;
    let mut args = vec!["-y".to_string()];
    args.extend(device.global_args());
    args.extend(device.input_args());
    args.extend([
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
    ]);

    let enabled: Vec<&TrackConfig> = tracks.iter().filter(|t| t.enabled).collect();
    let needs_mix =
        enabled.len() > 1 || (enabled.len() == 1 && (enabled[0].volume - 1.0).abs() >= 0.001);

    if needs_mix {
        args.extend([
            "-filter_complex".to_string(),
            amix_filter(&enabled),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "[aout]".to_string(),
        ]);
    } else if enabled.len() == 1 {
        args.extend([
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            format!("0:{}", enabled[0].stream_index),
        ]);
    } else if !tracks.is_empty() {
        // Tracks configured but all muted → video only.
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
    }
    // (empty `tracks` = no audio config → let ffmpeg map defaults)

    // VAAPI's encoder only accepts hardware frames; the simple -vf graph coexists
    // with the audio-only -filter_complex above.
    if let Some(suffix) = device.filter_suffix() {
        args.extend(["-vf".to_string(), suffix.to_string()]);
    }
    args.extend(device.video_quality_args());

    let audio_silenced = !tracks.is_empty() && enabled.is_empty();
    if audio_silenced {
        args.push("-an".to_string());
    } else {
        args.extend([
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            "192k".to_string(),
        ]);
    }

    args.extend([
        "-movflags".to_string(),
        "+faststart".to_string(),
        output.to_string(),
    ]);
    args
}

/// Target-size parameters for a sequence export. `pass` follows the compress-export
/// convention: CPU runs passes 1 and 2 against `passlog`; hardware devices only ever
/// run pass 2 (single-pass VBV).
#[derive(Debug, Clone)]
pub struct SequenceTarget {
    pub video_kbps: u32,
    pub audio_kbps: u32,
    pub pass: u8,
    pub passlog: String,
}

/// Normalization applied to every segment's audio before `concat`: the filter
/// requires identical sample rate/format/layout across all inputs, and mixed
/// sources (or amix's fltp output next to a passthrough stream) would fail.
const SEQ_AUDIO_NORM: &str =
    "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=PTS-STARTPTS";

/// The audio filter chain for one segment, honoring its per-track config with the
/// same semantics the preview mixer uses (spec §4.3 + 2026-07-05 design):
/// enabled tracks are volume-scaled and mixed into one stream; a configured-but-
/// fully-muted (or audioless) clip contributes silence so concat stays fed.
fn sequence_segment_audio(fc: &mut String, i: usize, s: &Segment) {
    let enabled: Vec<&TrackConfig> = s.tracks.iter().filter(|t| t.enabled).collect();
    if s.tracks.is_empty() {
        // No config (legacy caller) — take the default audio stream as before.
        fc.push_str(&format!("[{i}:a]{SEQ_AUDIO_NORM}[a{i}];"));
    } else if enabled.is_empty() {
        let dur = s.out_secs - s.in_secs;
        fc.push_str(&format!(
            "anullsrc=r=48000:cl=stereo,atrim=0:{dur:.3},asetpts=PTS-STARTPTS[a{i}];"
        ));
    } else if enabled.len() == 1 && (enabled[0].volume - 1.0).abs() < 0.001 {
        fc.push_str(&format!("[{i}:{}]{SEQ_AUDIO_NORM}[a{i}];", enabled[0].stream_index));
    } else {
        for (k, t) in enabled.iter().enumerate() {
            fc.push_str(&format!(
                "[{i}:{}]volume={:.3}[s{i}t{k}];",
                t.stream_index, t.volume
            ));
        }
        let labels: String = (0..enabled.len()).map(|k| format!("[s{i}t{k}]")).collect();
        fc.push_str(&format!(
            "{labels}amix=inputs={}:duration=longest:normalize=0,{SEQ_AUDIO_NORM}[a{i}];",
            enabled.len()
        ));
    }
}

/// Build ffmpeg args to stitch `segments` into one H.264/AAC MP4 (spec §4.3).
///
/// Each segment is fast-seeked (`-ss`/`-t`), scaled+letterboxed to the common
/// `width`×`height` (so mixed resolutions/aspect ratios concat cleanly), then
/// joined with `concat`. Per-segment audio mixes the tracks the preview played
/// (see `sequence_segment_audio`). An optional `music` overlay is mixed over the
/// whole sequence at its own volume — `-stream_loop -1` plus `amix duration=first`
/// loops it to the video length when requested.
///
/// `device` selects the video encoder (and hardware decode of the inputs);
/// `target` switches from quality mode (CRF-style) to a bitrate targeting a file
/// size, where pass 1 (CPU analysis) renders video only into the null sink.
pub fn build_sequence_args(
    segments: &[Segment],
    width: u32,
    height: u32,
    music: Option<&MusicOverlay>,
    output: &str,
    device: &ExportDevice,
    target: Option<&SequenceTarget>,
) -> Vec<String> {
    let analysis_pass = target.map(|t| t.pass == 1).unwrap_or(false);
    let mut args = vec!["-y".to_string()];
    args.extend(device.global_args());
    for s in segments {
        args.extend(device.input_args());
        args.push("-ss".to_string());
        args.push(format!("{:.3}", s.in_secs));
        args.push("-t".to_string());
        args.push(format!("{:.3}", s.out_secs - s.in_secs));
        args.push("-i".to_string());
        args.push(s.input.clone());
    }
    // Pass 1 analyzes video only — the music input would be dead weight.
    if let Some(m) = music.filter(|_| !analysis_pass) {
        if m.loop_to_length {
            args.push("-stream_loop".to_string());
            args.push("-1".to_string());
        }
        args.push("-i".to_string());
        args.push(m.path.clone());
    }

    let n = segments.len();
    let mut fc = String::new();
    for i in 0..n {
        // Letterbox each source into the target frame, resetting timestamps so
        // concat sees a clean 0-based stream.
        fc.push_str(&format!(
            "[{i}:v]scale={width}:{height}:force_original_aspect_ratio=decrease,\
             pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,setpts=PTS-STARTPTS[v{i}];"
        ));
        if !analysis_pass {
            sequence_segment_audio(&mut fc, i, &segments[i]);
        }
    }

    let mut video_out = "[cv]".to_string();
    let audio_out;
    if analysis_pass {
        let labels: String = (0..n).map(|i| format!("[v{i}]")).collect();
        fc.push_str(&format!("{labels}concat=n={n}:v=1:a=0[cv]"));
        audio_out = None;
    } else {
        let labels: String = (0..n).map(|i| format!("[v{i}][a{i}]")).collect();
        fc.push_str(&format!("{labels}concat=n={n}:v=1:a=1[cv][ca]"));
        audio_out = Some(if let Some(m) = music {
            let mi = n; // music is the input after all segments
            fc.push_str(&format!(";[{mi}:a]volume={:.3}[m];", m.volume));
            fc.push_str("[ca][m]amix=inputs=2:duration=first:normalize=0[outa]");
            "[outa]".to_string()
        } else {
            "[ca]".to_string()
        });
    }

    // VAAPI encodes hardware frames only — upload the concatenated output last.
    if let Some(suffix) = device.filter_suffix() {
        fc.push_str(&format!(";[cv]{suffix}[cvhw]"));
        video_out = "[cvhw]".to_string();
    }

    args.push("-filter_complex".to_string());
    args.push(fc);
    args.push("-map".to_string());
    args.push(video_out);
    if let Some(a) = &audio_out {
        args.push("-map".to_string());
        args.push(a.clone());
    }

    match target {
        None => args.extend(device.video_quality_args()),
        Some(t) => {
            args.extend(device.video_bitrate_args(t.video_kbps));
            if device.supports_two_pass() {
                args.extend([
                    "-pass".to_string(),
                    t.pass.to_string(),
                    "-passlogfile".to_string(),
                    t.passlog.clone(),
                ]);
            }
        }
    }

    if analysis_pass {
        args.extend(["-an", "-f", "mp4", null_sink()].map(String::from));
        return args;
    }

    let audio_kbps = target.map(|t| t.audio_kbps).unwrap_or(192);
    args.extend(
        [
            "-c:a",
            "aac",
            "-b:a",
            &format!("{audio_kbps}k"),
            "-movflags",
            "+faststart",
            output,
        ]
        .map(String::from),
    );
    args
}

/// Parses one ffmpeg `-progress` line into a 0.0–1.0 completion fraction against
/// `total_secs`. `out_time_us` is microseconds; `progress=end` marks completion.
/// Returns None for lines without a usable timestamp (e.g. `frame=…`, `N/A`).
pub fn parse_progress_line(line: &str, total_secs: f64) -> Option<f64> {
    let line = line.trim();
    if line == "progress=end" {
        return Some(1.0);
    }
    let micros: i64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    if total_secs <= 0.0 {
        return Some(0.0);
    }
    Some(((micros as f64 / 1_000_000.0) / total_secs).clamp(0.0, 1.0))
}

pub fn validate_trim(source: &str, output: &str, in_secs: f64, out_secs: f64) -> Result<(), String> {
    if output == source {
        return Err("Output path must differ from the source file.".into());
    }
    if !(in_secs < out_secs) {
        return Err("In point must be before out point.".into());
    }
    Ok(())
}

// ── Target-size compression (design spec §4.5) ────────────────────────────────────

/// Below this the picture is unusable at any resolution a game clip ships in.
pub const MIN_VIDEO_KBPS: u32 = 100;

/// Fraction of the target reserved for container overhead and rate-control drift.
const SIZE_SAFETY: f64 = 0.97;

/// Split a target file size into (video_kbps, audio_kbps) for a two-pass encode.
///
/// `target_total_bits = target_mb × 8 × 1024 × 1024 × safety`; audio gets a fixed
/// bitrate (stepped down for tight budgets) and video gets the remainder. Errors
/// when the remainder falls below MIN_VIDEO_KBPS — the pre-flight warning from the
/// design spec — so the user can shorten the clip or pick a larger target instead
/// of burning a two-pass encode on an unwatchable file.
pub fn compute_target_bitrates(target_mb: f64, duration_secs: f64) -> Result<(u32, u32), String> {
    if target_mb <= 0.0 {
        return Err("Target size must be greater than zero.".into());
    }
    if duration_secs <= 0.0 {
        return Err("Clip duration must be greater than zero.".into());
    }
    let total_kbps = target_mb * 8.0 * 1024.0 * 1024.0 * SIZE_SAFETY / 1000.0 / duration_secs;
    let audio_kbps: u32 = if total_kbps >= 512.0 {
        128
    } else if total_kbps >= 256.0 {
        96
    } else {
        64
    };
    let video_kbps = total_kbps - audio_kbps as f64;
    if video_kbps < MIN_VIDEO_KBPS as f64 {
        return Err(format!(
            "{target_mb} MB is too small for a {duration_secs:.1}s clip — video would drop below {MIN_VIDEO_KBPS} kbps. Shorten the clip or choose a bigger target."
        ));
    }
    Ok((video_kbps.floor() as u32, audio_kbps))
}

fn null_sink() -> &'static str {
    if cfg!(windows) {
        "NUL"
    } else {
        "/dev/null"
    }
}

/// Build ffmpeg args for one pass of an H.264/AAC target-size encode.
///
/// On CPU this is a two-pass x264 encode: pass 1 analyzes only (video discarded to
/// the OS null sink, audio dropped); pass 2 encodes the real output. Hardware
/// devices can't two-pass, so they run a single pass (callers pass `pass == 2`
/// only) with a VBV cap instead. Audio applies the same per-track semantics as
/// `build_trim_args` except it's always re-encoded to AAC — stream copy can't hit
/// a bitrate target.
pub fn build_compress_pass_args(
    input: &str,
    in_secs: f64,
    out_secs: f64,
    output: &str,
    tracks: &[TrackConfig],
    video_kbps: u32,
    audio_kbps: u32,
    pass: u8,
    passlog: &str,
    device: &ExportDevice,
) -> Vec<String> {
    let duration = out_secs - in_secs;
    let mut args = vec!["-y".to_string()];
    args.extend(device.global_args());
    args.extend(device.input_args());
    args.extend([
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
    ]);

    let enabled: Vec<&TrackConfig> = tracks.iter().filter(|t| t.enabled).collect();

    if pass == 1 {
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
        args.extend(device.video_bitrate_args(video_kbps));
        args.extend([
            "-pass".to_string(),
            "1".to_string(),
            "-passlogfile".to_string(),
            passlog.to_string(),
            "-an".to_string(),
            "-f".to_string(),
            "mp4".to_string(),
            null_sink().to_string(),
        ]);
        return args;
    }

    // Pass 2: audio mapping mirrors build_trim_args' mix/single-track semantics.
    let needs_mix = enabled.len() > 1
        || (enabled.len() == 1 && (enabled[0].volume - 1.0).abs() >= 0.001);
    if needs_mix {
        args.extend([
            "-filter_complex".to_string(),
            amix_filter(&enabled),
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            "[aout]".to_string(),
        ]);
    } else if enabled.len() == 1 {
        args.extend([
            "-map".to_string(),
            "0:v:0".to_string(),
            "-map".to_string(),
            format!("0:{}", enabled[0].stream_index),
        ]);
    } else if !tracks.is_empty() {
        // Tracks were configured but every one is muted → video only.
        args.extend(["-map".to_string(), "0:v:0".to_string()]);
    }
    // (empty `tracks` = clip with no audio config → let ffmpeg pick defaults)

    if let Some(suffix) = device.filter_suffix() {
        args.extend(["-vf".to_string(), suffix.to_string()]);
    }
    args.extend(device.video_bitrate_args(video_kbps));
    if device.supports_two_pass() {
        args.extend([
            "-pass".to_string(),
            "2".to_string(),
            "-passlogfile".to_string(),
            passlog.to_string(),
        ]);
    }

    let audio_silenced = !tracks.is_empty() && enabled.is_empty();
    if audio_silenced {
        args.push("-an".to_string());
    } else {
        args.extend([
            "-c:a".to_string(),
            "aac".to_string(),
            "-b:a".to_string(),
            format!("{audio_kbps}k"),
        ]);
    }

    args.extend([
        "-movflags".to_string(),
        "+faststart".to_string(),
        output.to_string(),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_input_seek_copy_args() {
        let args = build_trim_copy_args("in.mp4", 1.0, 3.5, "out.mp4");
        assert_eq!(
            args,
            vec!["-y", "-ss", "1.000", "-i", "in.mp4", "-t", "2.500", "-c", "copy", "out.mp4"]
        );
    }

    #[test]
    fn build_trim_args_single_enabled_track_at_unity_uses_stream_copy() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(args.contains(&"0:v:0".to_string()));
        assert!(args.contains(&"0:1".to_string()));
        assert!(args.contains(&"copy".to_string()));
        assert!(!args.contains(&"-filter_complex".to_string()));
    }

    #[test]
    fn build_trim_args_multiple_unity_tracks_still_mixes_to_one_stream() {
        // Two tracks both at 100% must still be combined into a single output
        // audio stream via amix — a multi-map stream-copy would leave every
        // track but the first silent in players that don't expose track pickers.
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 1.0 },
        ];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(args.contains(&"-filter_complex".to_string()));
        let fc_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[fc_idx + 1];
        assert!(filter.contains("amix=inputs=2"));
        assert!(filter.contains("[0:1]volume=1.000"));
        assert!(filter.contains("[0:2]volume=1.000"));
        assert!(args.contains(&"[aout]".to_string()));
        assert!(!args.contains(&"0:1".to_string())); // not stream-copy-mapped raw
    }

    #[test]
    fn build_trim_args_non_unity_volume_uses_amix() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 0.5 },
        ];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(args.contains(&"-filter_complex".to_string()));
        let fc_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[fc_idx + 1];
        assert!(filter.contains("amix=inputs=2"));
        assert!(filter.contains("volume=0.500"));
        assert!(args.contains(&"aac".to_string()));
    }

    #[test]
    fn build_trim_args_single_track_non_unity_no_amix() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 0.7 }];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        let fc_idx = args.iter().position(|a| a == "-filter_complex").unwrap();
        let filter = &args[fc_idx + 1];
        assert!(filter.contains("[aout]"));
        assert!(!filter.contains("amix"));
    }

    #[test]
    fn build_trim_args_muted_track_excluded() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: false, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 1.0 },
        ];
        let args = build_trim_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks);
        assert!(!args.contains(&"0:1".to_string()));
        assert!(args.contains(&"0:2".to_string()));
    }

    #[test]
    fn validate_trim_rejects_output_equal_to_source() {
        assert!(validate_trim("clip.mp4", "clip.mp4", 1.0, 3.5).is_err());
    }

    #[test]
    fn validate_trim_rejects_in_after_or_equal_out() {
        assert!(validate_trim("in.mp4", "out.mp4", 2.0, 2.0).is_err());
        assert!(validate_trim("in.mp4", "out.mp4", 5.0, 2.0).is_err());
    }

    #[test]
    fn validate_trim_accepts_valid_distinct_ordered_span() {
        assert!(validate_trim("in.mp4", "out.mp4", 1.0, 3.5).is_ok());
    }

    // ── Target-size compression ──────────────────────────────────────────────────

    #[test]
    fn compute_target_bitrates_splits_budget_between_video_and_audio() {
        // 10 MB over 60s: total ≈ 1356 kbps → 128k audio, remainder to video.
        let (v, a) = compute_target_bitrates(10.0, 60.0).unwrap();
        assert_eq!(a, 128);
        assert!((1200..=1240).contains(&v), "video kbps out of range: {v}");
    }

    #[test]
    fn compute_target_bitrates_steps_audio_down_for_tight_budgets() {
        // 8 MB over 300s: total ≈ 217 kbps → audio must drop to 64k.
        let (v, a) = compute_target_bitrates(8.0, 300.0).unwrap();
        assert_eq!(a, 64);
        assert!(v >= MIN_VIDEO_KBPS);
    }

    #[test]
    fn compute_target_bitrates_errors_below_video_floor() {
        // 8 MB over an hour is hopeless — must refuse, not encode garbage.
        let err = compute_target_bitrates(8.0, 3600.0).unwrap_err();
        assert!(err.contains("too small"), "unexpected message: {err}");
    }

    #[test]
    fn compute_target_bitrates_rejects_non_positive_inputs() {
        assert!(compute_target_bitrates(0.0, 60.0).is_err());
        assert!(compute_target_bitrates(10.0, 0.0).is_err());
    }

    #[test]
    fn compress_pass1_analyzes_video_only_into_null_sink() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args = build_compress_pass_args(
            "in.mp4", 1.0, 11.0, "out.mp4", &tracks, 1200, 128, 1, "/tmp/log", &ExportDevice::Cpu,
        );
        assert!(args.contains(&"-pass".to_string()));
        let p = args.iter().position(|a| a == "-pass").unwrap();
        assert_eq!(args[p + 1], "1");
        assert!(args.contains(&"-an".to_string()));
        assert!(args.contains(&"1200k".to_string()));
        assert!(!args.contains(&"out.mp4".to_string()));
        assert!(args.last().unwrap() == "/dev/null" || args.last().unwrap() == "NUL");
    }

    #[test]
    fn compress_pass2_single_unity_track_maps_stream_and_reencodes_aac() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args = build_compress_pass_args(
            "in.mp4", 1.0, 11.0, "out.mp4", &tracks, 1200, 128, 2, "/tmp/log", &ExportDevice::Cpu,
        );
        assert!(args.contains(&"0:1".to_string()));
        assert!(!args.contains(&"-filter_complex".to_string()));
        assert!(args.contains(&"aac".to_string()));
        assert!(args.contains(&"128k".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
        // Video must be re-encoded to hit the target — never stream-copied.
        assert!(args.contains(&"libx264".to_string()));
        assert!(!args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "copy"));
    }

    #[test]
    fn compress_pass2_multiple_tracks_use_amix() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 0.5 },
        ];
        let args = build_compress_pass_args(
            "in.mp4", 0.0, 10.0, "out.mp4", &tracks, 900, 96, 2, "/tmp/log", &ExportDevice::Cpu,
        );
        let fc = args.iter().position(|a| a == "-filter_complex").unwrap();
        assert!(args[fc + 1].contains("amix=inputs=2"));
        assert!(args.contains(&"[aout]".to_string()));
        assert!(args.contains(&"96k".to_string()));
    }

    #[test]
    fn compress_pass2_all_tracks_muted_drops_audio() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: false, volume: 1.0 }];
        let args = build_compress_pass_args(
            "in.mp4", 0.0, 10.0, "out.mp4", &tracks, 900, 128, 2, "/tmp/log", &ExportDevice::Cpu,
        );
        assert!(args.contains(&"-an".to_string()));
        assert!(!args.contains(&"aac".to_string()));
    }

    // ── Compatible (re-encode) export mode ───────────────────────────────────────

    #[test]
    fn compatible_reencodes_video_to_h264_with_faststart() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args = build_trim_compatible_args("in.mkv", 1.0, 6.0, "out.mp4", &tracks, &ExportDevice::Cpu);
        // Even a single unity track must re-encode video for universal playback.
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
        assert!(!args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "copy"));
        assert!(args.contains(&"aac".to_string())); // audio re-encoded too
        assert!(args.windows(2).any(|w| w[0] == "-movflags" && w[1] == "+faststart"));
        assert_eq!(args.last().unwrap(), "out.mp4");
        // Trim window preserved.
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[ss + 1], "1.000");
        assert!(args.windows(2).any(|w| w[0] == "-t" && w[1] == "5.000"));
    }

    #[test]
    fn compatible_mixes_multiple_tracks() {
        let tracks = vec![
            TrackConfig { stream_index: 1, enabled: true, volume: 1.0 },
            TrackConfig { stream_index: 2, enabled: true, volume: 0.5 },
        ];
        let args = build_trim_compatible_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks, &ExportDevice::Cpu);
        let fc = args.iter().position(|a| a == "-filter_complex").unwrap();
        assert!(args[fc + 1].contains("amix=inputs=2"));
        assert!(args.contains(&"[aout]".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
    }

    #[test]
    fn compatible_all_muted_drops_audio_but_still_reencodes_video() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: false, volume: 1.0 }];
        let args = build_trim_compatible_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks, &ExportDevice::Cpu);
        assert!(args.contains(&"-an".to_string()));
        assert!(!args.contains(&"aac".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
    }

    // ── FFmpeg -progress parsing ─────────────────────────────────────────────────

    #[test]
    fn progress_line_reports_fraction_of_duration() {
        // Halfway through a 10s job.
        let f = parse_progress_line("out_time_us=5000000", 10.0).unwrap();
        assert!((f - 0.5).abs() < 1e-6, "got {f}");
    }

    #[test]
    fn progress_end_marker_is_complete() {
        assert_eq!(parse_progress_line("progress=end", 10.0), Some(1.0));
    }

    #[test]
    fn progress_line_clamps_and_ignores_irrelevant_lines() {
        // Overshoot (encoder can report slightly past duration) clamps to 1.0.
        assert_eq!(parse_progress_line("out_time_us=11000000", 10.0), Some(1.0));
        // Non-timestamp lines yield nothing.
        assert_eq!(parse_progress_line("frame=42", 10.0), None);
        assert_eq!(parse_progress_line("out_time_us=N/A", 10.0), None);
    }

    // ── Sequence stitching + music overlay (spec §4.3) ───────────────────────────

    fn seg(input: &str, in_s: f64, out_s: f64) -> Segment {
        Segment { input: input.to_string(), in_secs: in_s, out_secs: out_s, tracks: vec![] }
    }

    fn seg_tracks(input: &str, in_s: f64, out_s: f64, tracks: Vec<TrackConfig>) -> Segment {
        Segment { input: input.to_string(), in_secs: in_s, out_secs: out_s, tracks }
    }

    #[test]
    fn sequence_two_segments_concats_and_reencodes() {
        let segs = vec![seg("a.mp4", 0.0, 2.0), seg("b.mp4", 1.0, 4.0)];
        let args = build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Cpu, None);
        // One -i per segment, each fast-seeked and duration-limited.
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 2);
        assert_eq!(args.iter().filter(|a| *a == "-ss").count(), 2);
        let fc = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(fc.contains("concat=n=2:v=1:a=1"));
        assert!(fc.contains("[v0][a0][v1][a1]concat"));
        assert!(fc.contains("scale=1280:720"));
        // Straight concat output mapped, no music mixing.
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "[cv]"));
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "[ca]"));
        assert!(!fc.contains("amix"));
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
        assert!(args.windows(2).any(|w| w[0] == "-movflags" && w[1] == "+faststart"));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn sequence_single_segment_still_valid() {
        let args = build_sequence_args(
            &[seg("a.mp4", 0.5, 3.5)], 1920, 1080, None, "out.mp4", &ExportDevice::Cpu, None,
        );
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 1);
        let fc = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(fc.contains("concat=n=1:v=1:a=1"));
    }

    #[test]
    fn sequence_with_looped_music_mixes_and_stream_loops() {
        let segs = vec![seg("a.mp4", 0.0, 2.0)];
        let music = MusicOverlay { path: "song.mp3".into(), volume: 0.4, loop_to_length: true };
        let args =
            build_sequence_args(&segs, 1280, 720, Some(&music), "out.mp4", &ExportDevice::Cpu, None);
        // Music is a third input, looped so it fills the whole sequence.
        assert!(args.windows(2).any(|w| w[0] == "-stream_loop" && w[1] == "-1"));
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 2); // 1 clip + music
        let fc = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(fc.contains("[1:a]volume=0.400")); // music input index = segments.len()
        assert!(fc.contains("amix=inputs=2:duration=first"));
        assert!(fc.contains("[outa]"));
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "[outa]"));
    }

    #[test]
    fn sequence_with_unlooped_music_has_no_stream_loop() {
        let segs = vec![seg("a.mp4", 0.0, 2.0)];
        let music = MusicOverlay { path: "song.mp3".into(), volume: 1.0, loop_to_length: false };
        let args =
            build_sequence_args(&segs, 1280, 720, Some(&music), "out.mp4", &ExportDevice::Cpu, None);
        assert!(!args.contains(&"-stream_loop".to_string()));
        let fc = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(fc.contains("amix=inputs=2:duration=first"));
    }

    // ── Per-segment audio tracks in sequences (2026-07-05 design) ────────────────

    fn track(idx: u32, enabled: bool, volume: f64) -> TrackConfig {
        TrackConfig { stream_index: idx, enabled, volume }
    }

    fn sequence_fc(args: &[String]) -> String {
        args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1].clone()
    }

    #[test]
    fn sequence_segment_mixes_all_enabled_tracks_not_just_the_first() {
        // The reported bug: two tracks audible in preview, but only stream 1 made
        // it into the exported sequence. Both must be mixed per segment.
        let segs = vec![seg_tracks(
            "a.mp4", 0.0, 2.0,
            vec![track(1, true, 1.0), track(2, true, 1.0)],
        )];
        let args = build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Cpu, None);
        let fc = sequence_fc(&args);
        assert!(fc.contains("[0:1]volume=1.000[s0t0]"), "fc: {fc}");
        assert!(fc.contains("[0:2]volume=1.000[s0t1]"), "fc: {fc}");
        assert!(fc.contains("[s0t0][s0t1]amix=inputs=2"), "fc: {fc}");
        assert!(!fc.contains("[0:a]"), "must not fall back to the default stream: {fc}");
    }

    #[test]
    fn sequence_segment_single_unity_track_maps_that_stream_directly() {
        let segs = vec![seg_tracks("a.mp4", 0.0, 2.0, vec![track(2, true, 1.0), track(1, false, 1.0)])];
        let args = build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Cpu, None);
        let fc = sequence_fc(&args);
        assert!(fc.contains("[0:2]aresample=48000"), "fc: {fc}");
        assert!(!fc.contains("amix=inputs=1"), "fc: {fc}");
        assert!(!fc.contains("[0:1]"), "muted track must be excluded: {fc}");
    }

    #[test]
    fn sequence_segment_honors_non_unity_volume() {
        let segs = vec![seg_tracks("a.mp4", 0.0, 2.0, vec![track(1, true, 0.5)])];
        let args = build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Cpu, None);
        let fc = sequence_fc(&args);
        assert!(fc.contains("[0:1]volume=0.500"), "fc: {fc}");
    }

    #[test]
    fn sequence_segment_all_muted_contributes_silence() {
        let segs = vec![
            seg_tracks("a.mp4", 0.0, 2.5, vec![track(1, false, 1.0)]),
            seg_tracks("b.mp4", 0.0, 1.0, vec![track(1, true, 1.0)]),
        ];
        let args = build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Cpu, None);
        let fc = sequence_fc(&args);
        assert!(fc.contains("anullsrc=r=48000:cl=stereo,atrim=0:2.500"), "fc: {fc}");
        assert!(fc.contains("concat=n=2:v=1:a=1"), "fc: {fc}");
    }

    #[test]
    fn sequence_normalizes_audio_before_concat() {
        // concat needs identical sample rate/format/layout across segments.
        let segs = vec![seg("a.mp4", 0.0, 2.0), seg_tracks("b.mp4", 0.0, 1.0, vec![track(1, true, 1.0)])];
        let args = build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Cpu, None);
        let fc = sequence_fc(&args);
        assert_eq!(fc.matches("aresample=48000").count(), 2, "fc: {fc}");
        assert!(fc.contains("aformat=sample_fmts=fltp:channel_layouts=stereo"), "fc: {fc}");
    }

    // ── Export devices (GPU encode/decode, 2026-07-05 design) ────────────────────

    #[test]
    fn export_device_from_id_falls_back_to_cpu() {
        assert_eq!(ExportDevice::from_id("vaapi"), ExportDevice::Vaapi);
        assert_eq!(ExportDevice::from_id("nvenc"), ExportDevice::Nvenc);
        assert_eq!(ExportDevice::from_id("cpu"), ExportDevice::Cpu);
        assert_eq!(ExportDevice::from_id("banana"), ExportDevice::Cpu);
        assert_eq!(ExportDevice::from_id(""), ExportDevice::Cpu);
    }

    #[test]
    fn compatible_vaapi_inits_device_uploads_frames_and_hw_decodes() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args =
            build_trim_compatible_args("in.mp4", 0.0, 5.0, "out.mp4", &tracks, &ExportDevice::Vaapi);
        assert!(args.windows(2).any(|w| w[0] == "-init_hw_device" && w[1] == "vaapi=hw"));
        assert!(args.windows(2).any(|w| w[0] == "-hwaccel" && w[1] == "vaapi"));
        assert!(args.windows(2).any(|w| w[0] == "-vf" && w[1] == "format=nv12,hwupload"));
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "h264_vaapi"));
        assert!(!args.contains(&"libx264".to_string()));
        // hwaccel is an input option — it must precede the -i.
        let hw = args.iter().position(|a| a == "-hwaccel").unwrap();
        let input = args.iter().position(|a| a == "-i").unwrap();
        assert!(hw < input);
    }

    #[test]
    fn compress_on_gpu_is_single_pass_with_vbv_cap() {
        let tracks = vec![TrackConfig { stream_index: 1, enabled: true, volume: 1.0 }];
        let args = build_compress_pass_args(
            "in.mp4", 0.0, 10.0, "out.mp4", &tracks, 1200, 128, 2, "/tmp/log", &ExportDevice::Nvenc,
        );
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "h264_nvenc"));
        assert!(args.windows(2).any(|w| w[0] == "-b:v" && w[1] == "1200k"));
        assert!(args.windows(2).any(|w| w[0] == "-maxrate" && w[1] == "1200k"));
        assert!(args.windows(2).any(|w| w[0] == "-bufsize" && w[1] == "2400k"));
        assert!(!args.contains(&"-pass".to_string()));
        assert!(!args.contains(&"-passlogfile".to_string()));
    }

    #[test]
    fn sequence_vaapi_uploads_concat_output_and_maps_it() {
        let segs = vec![seg("a.mp4", 0.0, 2.0)];
        let args = build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Vaapi, None);
        let fc = sequence_fc(&args);
        assert!(fc.contains(";[cv]format=nv12,hwupload[cvhw]"), "fc: {fc}");
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "[cvhw]"));
        assert!(!args.windows(2).any(|w| w[0] == "-map" && w[1] == "[cv]"));
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "h264_vaapi"));
    }

    // ── Sequence target size (same Size dropdown as single clips) ────────────────

    #[test]
    fn sequence_target_cpu_pass1_analyzes_video_only() {
        let segs = vec![seg("a.mp4", 0.0, 2.0), seg("b.mp4", 0.0, 3.0)];
        let music = MusicOverlay { path: "song.mp3".into(), volume: 0.5, loop_to_length: true };
        let target = SequenceTarget {
            video_kbps: 1000, audio_kbps: 128, pass: 1, passlog: "/tmp/log".into(),
        };
        let args = build_sequence_args(
            &segs, 1280, 720, Some(&music), "out.mp4", &ExportDevice::Cpu, Some(&target),
        );
        let fc = sequence_fc(&args);
        assert!(fc.contains("concat=n=2:v=1:a=0"), "fc: {fc}");
        assert!(!fc.contains("[a0]"), "no audio chains in analysis: {fc}");
        assert!(args.contains(&"-an".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "-pass" && w[1] == "1"));
        // Music input skipped in the analysis pass.
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 2);
        assert!(args.last().unwrap() == "/dev/null" || args.last().unwrap() == "NUL");
    }

    #[test]
    fn sequence_target_cpu_pass2_writes_real_output_at_bitrate() {
        let segs = vec![seg("a.mp4", 0.0, 2.0)];
        let target = SequenceTarget {
            video_kbps: 1000, audio_kbps: 96, pass: 2, passlog: "/tmp/log".into(),
        };
        let args =
            build_sequence_args(&segs, 1280, 720, None, "out.mp4", &ExportDevice::Cpu, Some(&target));
        assert!(args.windows(2).any(|w| w[0] == "-b:v" && w[1] == "1000k"));
        assert!(args.windows(2).any(|w| w[0] == "-pass" && w[1] == "2"));
        assert!(args.windows(2).any(|w| w[0] == "-b:a" && w[1] == "96k"));
        assert!(!args.contains(&"-crf".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn sequence_target_on_gpu_is_single_pass() {
        let segs = vec![seg("a.mp4", 0.0, 2.0)];
        let target = SequenceTarget {
            video_kbps: 1000, audio_kbps: 128, pass: 2, passlog: "/tmp/log".into(),
        };
        let args = build_sequence_args(
            &segs, 1280, 720, None, "out.mp4", &ExportDevice::Nvenc, Some(&target),
        );
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "h264_nvenc"));
        assert!(args.windows(2).any(|w| w[0] == "-maxrate" && w[1] == "1000k"));
        assert!(!args.contains(&"-pass".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }
}
