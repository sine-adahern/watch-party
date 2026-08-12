//! WebSocket endpoint: parse [`shared::ClientMsg`], drive the room, and stream
//! [`shared::ServerMsg`] back. One task reads the socket; a second drains this
//! connection's channel and writes to it.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::AppState;
use shared::{ClientMsg, PeerId, ServerMsg};

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();

    // Writer: this peer's outbound messages -> JSON text on the socket.
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let Ok(json) = serde_json::to_string(&msg) else { continue };
            if sink.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    let mut peer_id: Option<PeerId> = None;
    let mut room: Option<String> = None;

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(client_msg) = serde_json::from_str::<ClientMsg>(&text) else {
            continue;
        };

        match client_msg {
            ClientMsg::Join { room: r, name, .. } => {
                let id = new_peer_id();
                state.rooms.join(&r, id.clone(), name, tx.clone());
                peer_id = Some(id);
                room = Some(r);
            }
            ClientMsg::Play { at } => {
                if let Some(r) = &room {
                    state.rooms.set_play(r, true, at);
                }
            }
            ClientMsg::Pause { at } => {
                if let Some(r) = &room {
                    state.rooms.set_play(r, false, at);
                }
            }
            ClientMsg::Seek { to } => {
                if let Some(r) = &room {
                    state.rooms.seek(r, to);
                }
            }
            ClientMsg::LoadVideo { video_id } => {
                if let Some(r) = &room {
                    state.rooms.load_video(r, video_id);
                }
            }
            ClientMsg::Signal { to, kind, payload } => {
                if let (Some(r), Some(from)) = (&room, &peer_id) {
                    state.rooms.relay_signal(r, from.clone(), &to, kind, payload);
                }
            }
        }
    }

    if let (Some(r), Some(id)) = (room, peer_id) {
        state.rooms.leave(&r, &id);
    }
    writer.abort();
}

fn new_peer_id() -> String {
    let mut buf = [0u8; 8];
    use std::io::Read;
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        buf.copy_from_slice(&(n as u64).to_le_bytes());
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
