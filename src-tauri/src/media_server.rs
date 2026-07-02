use axum::{
    extract::Query,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};
use tower_http::cors::CorsLayer;

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

async fn serve_file(headers: HeaderMap, Query(q): Query<FileQuery>) -> impl IntoResponse {
    let path = &q.path;

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            let status = if e.kind() == std::io::ErrorKind::NotFound {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::FORBIDDEN
            };
            return (status, axum::body::Body::empty()).into_response();
        }
    };

    let len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, axum::body::Body::empty()).into_response()
        }
    };

    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    let mime = mime_for_ext(&ext);

    if let Some(range_str) = headers.get("range").and_then(|v| v.to_str().ok()) {
        if let Some((start, end)) = parse_byte_range(range_str, len) {
            let count = end - start + 1;
            let mut buf = vec![0u8; count as usize];
            if file.seek(SeekFrom::Start(start)).is_ok() && file.read_exact(&mut buf).is_ok() {
                return (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        (header::CONTENT_TYPE, mime.to_string()),
                        (header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}")),
                        (header::CONTENT_LENGTH, count.to_string()),
                        (header::ACCEPT_RANGES, "bytes".to_string()),
                    ],
                    buf,
                )
                    .into_response();
            }
        }
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{len}"))],
        )
            .into_response();
    }

    let mut buf = Vec::with_capacity(len as usize);
    let _ = file.read_to_end(&mut buf);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::CONTENT_LENGTH, len.to_string()),
            (header::ACCEPT_RANGES, "bytes".to_string()),
        ],
        buf,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_for_ext_covers_video_audio_and_thumbnail_images() {
        assert_eq!(mime_for_ext("mp4"), "video/mp4");
        assert_eq!(mime_for_ext("mov"), "video/mp4");
        assert_eq!(mime_for_ext("m4a"), "audio/mp4");
        assert_eq!(mime_for_ext("mkv"), "video/x-matroska");
        // thumbnails must be served as real images so <img> renders them
        assert_eq!(mime_for_ext("jpg"), "image/jpeg");
        assert_eq!(mime_for_ext("jpeg"), "image/jpeg");
        assert_eq!(mime_for_ext("png"), "image/png");
        assert_eq!(mime_for_ext("unknown"), "application/octet-stream");
    }
}

/// Maps a lowercased file extension to a Content-Type. `.mov` is advertised as
/// `video/mp4` (ISOBMFF-compatible) so every WebView accepts it; `.m4a`/`.aac`
/// as `audio/mp4` for extracted audio tracks; image types cover thumbnails.
fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "mp4" | "m4v" | "mov" => "video/mp4",
        "m4a" | "aac" => "audio/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "flv" => "video/x-flv",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn parse_byte_range(range_str: &str, len: u64) -> Option<(u64, u64)> {
    let s = range_str.strip_prefix("bytes=")?;
    let dash = s.find('-')?;
    let start: u64 = s[..dash].trim().parse().ok()?;
    let end: u64 = s[dash + 1..].trim().parse().unwrap_or(len.saturating_sub(1));
    if start >= len {
        return None;
    }
    Some((start, end.min(len - 1)))
}

/// Starts the local media HTTP server and returns the port it bound to.
/// The server serves arbitrary local files at GET /file?path=<absolute-path>.
/// This is the delivery mechanism for <video> playback: real HTTP works
/// uniformly on Linux (WebKitGTK), Windows (WebView2), and macOS (WKWebView),
/// whereas custom URI scheme protocols are unreliable for media in WebKitGTK 2.52+.
pub fn start() -> u16 {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("failed to bind media HTTP server");
    std_listener.set_nonblocking(true).unwrap();
    let port = std_listener.local_addr().unwrap().port();

    let router = Router::new()
        .route("/file", get(serve_file))
        .layer(CorsLayer::permissive());

    tauri::async_runtime::spawn(async move {
        let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
        axum::serve(listener, router).await.unwrap();
    });

    port
}
