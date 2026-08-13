//! Upload, list, delete, and serve home videos. Metadata is in SQLite (see
//! `db.rs`); files stream to disk as `media/{id}.mp4`. If `ffmpeg` is on PATH,
//! uploads are remuxed with `+faststart` so playback/seek start instantly.

use std::path::Path;

use axum::{
    extract::{Path as AxPath, Request, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::{db, AppState};
use shared::Video;

/// Reject uploads larger than this (4 GiB) — generous for home clips.
const MAX_UPLOAD: usize = 4 * 1024 * 1024 * 1024;

/// `GET /api/videos`
pub async fn list_videos(State(state): State<AppState>) -> Json<Vec<Video>> {
    let conn = state.db.lock().unwrap();
    Json(db::list(&conn))
}

/// `POST /api/upload` — raw bytes as body, original name in `x-filename`.
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
    drop(file);

    if wrote == 0 {
        let _ = tokio::fs::remove_file(&file_path).await;
        return Err((StatusCode::BAD_REQUEST, "empty upload".into()));
    }

    // Best-effort: move the moov atom to the front for instant start/seek.
    faststart(&file_path).await;

    let video = Video {
        id,
        name,
        uploaded_at: now_secs(),
    };
    {
        let conn = state.db.lock().unwrap();
        db::insert(&conn, &video);
    }
    tracing::info!("stored {} ({} bytes) as {}", video.name, wrote, video.id);
    Ok(Json(video))
}

/// `DELETE /api/videos/{id}` — remove the row and the file.
pub async fn delete_video(
    State(state): State<AppState>,
    AxPath(id): AxPath<String>,
) -> StatusCode {
    let removed = {
        let conn = state.db.lock().unwrap();
        db::delete(&conn, &id)
    };
    let _ = tokio::fs::remove_file(state.media_dir.join(format!("{id}.mp4"))).await;
    if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Remux in place with `+faststart` if ffmpeg is available. Any failure (incl.
/// ffmpeg not installed) leaves the original file untouched.
async fn faststart(path: &Path) {
    let tmp = path.with_extension("fast.mp4");
    let result = tokio::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(path)
        .args(["-c", "copy", "-movflags", "+faststart"])
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    match result {
        Ok(status) if status.success() => {
            let _ = tokio::fs::rename(&tmp, path).await;
        }
        _ => {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
    }
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn new_id() -> String {
    let mut buf = [0u8; 16];
    use std::io::Read;
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
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
