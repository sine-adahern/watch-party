//! Upload, list, and index home videos on the local filesystem.
//!
//! Metadata lives in a small `media/videos.json` file plus an in-memory list.
//! That's deliberately dependency-free for Phase 1; swap in `sqlx`/SQLite in
//! Phase 4 without touching the handlers' signatures.

use std::path::Path;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::AppState;
use shared::Video;

const INDEX_FILE: &str = "videos.json";
/// Reject uploads larger than this (4 GiB) — generous for home clips.
const MAX_UPLOAD: usize = 4 * 1024 * 1024 * 1024;

pub async fn load_index(media_dir: &Path) -> Vec<Video> {
    match tokio::fs::read(media_dir.join(INDEX_FILE)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn save_index(media_dir: &Path, videos: &[Video]) {
    if let Ok(json) = serde_json::to_vec_pretty(videos) {
        let _ = tokio::fs::write(media_dir.join(INDEX_FILE), json).await;
    }
}

/// `GET /api/videos` -> the current library.
pub async fn list_videos(State(state): State<AppState>) -> Json<Vec<Video>> {
    Json(state.videos.read().await.clone())
}

/// `POST /api/upload` with the raw file bytes as the body and the original
/// filename in an `x-filename` header. Streams to disk as `media/{id}.mp4`.
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> Result<Json<Video>, (StatusCode, String)> {
    let id = new_id();
    let name = headers
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{id}.mp4"));

    let file_path = state.media_dir.join(format!("{id}.mp4"));
    let mut file = tokio::fs::File::create(&file_path).await.map_err(internal)?;

    let mut stream = request.into_body().into_data_stream();
    let mut wrote: usize = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        wrote += chunk.len();
        if wrote > MAX_UPLOAD {
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "file too large".into()));
        }
        file.write_all(&chunk).await.map_err(internal)?;
    }
    file.flush().await.map_err(internal)?;

    if wrote == 0 {
        let _ = tokio::fs::remove_file(&file_path).await;
        return Err((StatusCode::BAD_REQUEST, "empty upload".into()));
    }

    let video = Video {
        id,
        name,
        uploaded_at: now_secs(),
    };
    {
        let mut videos = state.videos.write().await;
        videos.push(video.clone());
        save_index(&state.media_dir, &videos).await;
    }
    tracing::info!("stored {} ({} bytes) as {}", video.name, wrote, video.id);
    Ok(Json(video))
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn new_id() -> String {
    // 16 random bytes from the OS as lowercase hex — no external crates.
    let mut buf = [0u8; 16];
    use std::io::Read;
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
        // Fallback: time + pid. Fine for a small self-hosted app.
        let seed = (now_secs() as u128) ^ ((std::process::id() as u128) << 64);
        buf.copy_from_slice(&seed.to_le_bytes());
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
