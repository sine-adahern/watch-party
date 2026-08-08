//! Thin HTTP client for the backend. Same-origin in production; in dev,
//! `trunk serve` proxies these paths to the Axum server (see Trunk.toml).

use gloo_net::http::Request;
use shared::Video;
use wasm_bindgen::JsValue;
use web_sys::File;

/// `GET /api/videos`
pub async fn fetch_videos() -> Result<Vec<Video>, String> {
    Request::get("/api/videos")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<Vec<Video>>()
        .await
        .map_err(|e| e.to_string())
}

/// `POST /api/upload` — the `File` (a Blob) is sent as the raw body, with the
/// original name in a header so the server can display it later.
pub async fn upload(file: File) -> Result<Video, String> {
    let name = file.name();
    let body: JsValue = file.into();
    let resp = Request::post("/api/upload")
        .header("x-filename", &name)
        .body(body)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        return Err(format!("server returned {}", resp.status()));
    }
    resp.json::<Video>().await.map_err(|e| e.to_string())
}
