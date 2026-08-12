//! Phase 1 backend: upload home videos, serve them (with seek support),
//! and list them. A `/ws` echo endpoint and a `rooms` registry are stubbed
//! in so Phase 2 (playback sync) drops in without restructuring.

mod media;
mod rooms;
mod ws;

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use tokio::sync::RwLock;
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::rooms::RoomRegistry;

/// Shared application state. Cheap to clone (everything is behind an `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub media_dir: PathBuf,
    pub videos: Arc<RwLock<Vec<shared::Video>>>,
    pub rooms: Arc<RoomRegistry>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let media_dir = PathBuf::from(std::env::var("MEDIA_DIR").unwrap_or_else(|_| "media".into()));
    tokio::fs::create_dir_all(&media_dir)
        .await
        .expect("create media dir");

    let videos = media::load_index(&media_dir).await;
    tracing::info!("loaded {} video(s) from {}", videos.len(), media_dir.display());

    let state = AppState {
        media_dir: media_dir.clone(),
        videos: Arc::new(RwLock::new(videos)),
        rooms: Arc::new(RoomRegistry::new()),
    };

    // Phase 2: nudge playing rooms back into sync every few seconds.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3));
            loop {
                tick.tick().await;
                state.rooms.heartbeat();
            }
        });
    }

    let app = Router::new()
        .route("/api/videos", get(media::list_videos))
        .route(
            "/api/upload",
            // Uploads stream to disk; disable the small default body cap and
            // enforce our own limit inside the handler.
            post(media::upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/ws", get(ws::handler))
        // Uploaded videos. `ServeDir` answers HTTP range requests, so the
        // browser's <video> element can seek for free.
        .nest_service("/media", ServeDir::new(&media_dir))
        // The built frontend (after `trunk build`). In dev, `trunk serve`
        // hosts the UI and proxies /api + /media + /ws here, so this fallback
        // is only used in production.
        .fallback_service(ServeDir::new("web/dist"))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
