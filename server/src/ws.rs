//! WebSocket endpoint.
//!
//! Phase 1: a plain echo, just to prove the upgrade + proxy path works. Phase 2
//! replaces `handle_socket` with: parse [`shared::ClientMsg`], join the room in
//! the [`crate::rooms::RoomRegistry`], and forward [`shared::ServerMsg`] updates.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures_util::StreamExt;

use crate::AppState;

pub async fn handler(ws: WebSocketUpgrade, State(_state): State<AppState>) -> Response {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    while let Some(Ok(msg)) = socket.next().await {
        match msg {
            Message::Text(text) => {
                if socket.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}
