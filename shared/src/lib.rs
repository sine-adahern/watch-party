//! Wire protocol + shared types for the watch-party app.
//!
//! This crate compiles into BOTH the Axum server and the WASM frontend, so the
//! two sides can never disagree about message shapes — a mismatch is a compile
//! error, not a runtime bug. Phase 1 only uses [`Video`]; the message enums are
//! here so Phase 2 (sync) and Phase 3 (calls) drop straight in.

use serde::{Deserialize, Serialize};

/// Metadata for one uploaded home video. Returned by `GET /api/videos`.
/// The file itself lives on disk at `media/{id}.mp4`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub name: String,
    pub uploaded_at: i64, // unix seconds
}

/// A participant in a room. The server assigns [`PeerId`] on join.
pub type PeerId = String;
pub type RoomId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: PeerId,
    pub name: String,
}

/// Which kind of WebRTC signaling payload is being relayed (Phase 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Offer,
    Answer,
    IceCandidate,
}

/// Messages the browser sends UP to the server over the WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Join a room and start receiving state + peer events.
    Join {
        room: RoomId,
        name: String,
        video_id: Option<String>,
    },
    /// Playback controls (Phase 2). Positions are in seconds.
    Play { at: f64 },
    Pause { at: f64 },
    Seek { to: f64 },
    /// Switch everyone to a different video.
    LoadVideo { video_id: String },
    /// WebRTC signaling relayed to one specific peer (Phase 3).
    Signal {
        to: PeerId,
        kind: SignalKind,
        /// Opaque payload: an SDP string for Offer/Answer, or a JSON ICE
        /// candidate for IceCandidate.
        payload: String,
    },
}

/// Messages the server pushes DOWN to the browser over the WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent once on join: your identity plus who's already here.
    Welcome { you: PeerId, peers: Vec<Peer> },
    /// Authoritative playback state. Clients reconcile against this:
    /// expected = position + (now - server_time) when `playing`.
    State {
        playing: bool,
        position: f64,
        server_time: f64,
        rate: f64,
        video_id: Option<String>,
    },
    PeerJoined { peer: Peer },
    PeerLeft { id: PeerId },
    /// A signaling payload from another peer (Phase 3).
    Signal {
        from: PeerId,
        kind: SignalKind,
        payload: String,
    },
}
