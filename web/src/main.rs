//! Phase 1 frontend (Rust -> WASM via Leptos, client-side rendered).
//! Upload a home video, list the library, play a chosen one. The WebSocket
//! sync layer (Phase 2) plugs into `crate::api` + the shared protocol.

mod api;
mod call;
mod app;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount_to_body(app::App);
}
