use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMeta {
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub container: String,
    pub video_codec: String,
    pub audio_tracks: Vec<AudioTrack>,
}

pub fn parse_ffprobe_json(json: &str) -> Result<ClipMeta, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let streams = v["streams"].as_array().ok_or("no streams in ffprobe output")?;

    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or("no video stream found")?;
    let width = video["width"].as_u64().ok_or("video stream missing width")? as u32;
    let height = video["height"].as_u64().ok_or("video stream missing height")? as u32;
    let video_codec = video["codec_name"].as_str().unwrap_or("unknown").to_string();

    let audio_tracks = streams
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .map(|s| AudioTrack {
            index: s["index"].as_u64().unwrap_or(0) as u32,
            codec: s["codec_name"].as_str().unwrap_or("unknown").to_string(),
            channels: s["channels"].as_u64().unwrap_or(0) as u32,
            language: s["tags"]["language"].as_str().map(|x| x.to_string()),
        })
        .collect();

    let container = v["format"]["format_name"].as_str().unwrap_or("unknown").to_string();
    let duration_secs = v["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .ok_or("format.duration missing or unparseable")?;

    Ok(ClipMeta { duration_secs, width, height, container, video_codec, audio_tracks })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "streams": [
        { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080 },
        { "index": 1, "codec_type": "audio", "codec_name": "aac", "channels": 2, "tags": { "language": "eng" } },
        { "index": 2, "codec_type": "audio", "codec_name": "aac", "channels": 2 }
      ],
      "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.345" }
    }"#;

    #[test]
    fn parses_video_audio_and_format() {
        let m = parse_ffprobe_json(SAMPLE).unwrap();
        assert_eq!(m.width, 1920);
        assert_eq!(m.height, 1080);
        assert_eq!(m.video_codec, "h264");
        assert_eq!(m.container, "mov,mp4,m4a,3gp,3g2,mj2");
        assert!((m.duration_secs - 12.345).abs() < 1e-6);
        assert_eq!(m.audio_tracks.len(), 2);
        assert_eq!(m.audio_tracks[0].language.as_deref(), Some("eng"));
        assert_eq!(m.audio_tracks[1].language, None);
        assert_eq!(m.audio_tracks[0].channels, 2);
    }
}
