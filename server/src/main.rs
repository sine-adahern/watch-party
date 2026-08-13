//! Watch-party backend.
//! Phase 1: upload + range-served video. Phase 2: room sync over `/ws`.
//! Phase 3: WebRTC signaling relay. Phase 4: SQLite library + delete + faststart.

mod db;
mod media;
mod rooms;
mod ws;

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use rusqlite::Connection;
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::rooms::RoomRegistry;

#[derive(Clone)]
pub struct AppState {
    pub media_dir: PathBuf,
    pub db: Arc<Mutex<Connection>>,
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

    let db = Arc::new(db::open(&media_dir.join("library.db")));
    tracing::info!("library: {} video(s)", db::list(&db.lock().unwrap()).len());

    let state = AppState {
        media_dir: media_dir.clone(),
        db,
        rooms: Arc::new(RoomRegistry::new()),
    };

    // Nudge playing rooms back into sync every few seconds.
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
            post(media::upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/videos/:id", delete(media::delete_video))
        .route("/ws", get(ws::handler))
        // Uploaded videos, with HTTP range (seek) support for free.
        .nest_service("/media", ServeDir::new(&media_dir))
        // Built frontend in production; in dev, trunk serves + proxies here.
        .fallback_service(ServeDir::new("web/dist"))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
