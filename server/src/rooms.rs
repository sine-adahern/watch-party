//! Room registry — the heart of Phase 2 (playback sync).
//!
//! Each watch party is a [`Room`] holding the authoritative playback state plus
//! a channel to every connected peer. Any control action updates the state,
//! stamps it with the server clock, and fans a [`ServerMsg::State`] out to
//! everyone. Clients reconcile against that. A short `std::sync::Mutex` guards
//! the map; it is never held across an `.await`, so it stays simple and correct.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use shared::{Peer, PeerId, RoomId, ServerMsg, SignalKind};

/// Per-connection sender. The WebSocket writer task drains the matching
/// receiver and serializes each message to the socket.
pub type Tx = mpsc::UnboundedSender<ServerMsg>;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Clone)]
struct Playback {
    playing: bool,
    position: f64,   // video position (s) at `updated_at`
    updated_at: f64, // server unix time when the anchor was set
    rate: f64,
    video_id: Option<String>,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: false,
            position: 0.0,
            updated_at: now(),
            rate: 1.0,
            video_id: None,
        }
    }
}

impl Playback {
    /// Where the video *should* be right now, extrapolating from the anchor.
    fn current_position(&self) -> f64 {
        if self.playing {
            self.position + (now() - self.updated_at) * self.rate
        } else {
            self.position
        }
    }

    fn to_state(&self) -> ServerMsg {
        ServerMsg::State {
            playing: self.playing,
            position: self.current_position(),
            server_time: now(),
            rate: self.rate,
            video_id: self.video_id.clone(),
        }
    }
}

struct PeerHandle {
    name: String,
    tx: Tx,
}

#[derive(Default)]
struct Room {
    state: Playback,
    peers: HashMap<PeerId, PeerHandle>,
}

impl Room {
    fn broadcast(&self, msg: &ServerMsg) {
        for h in self.peers.values() {
            let _ = h.tx.send(msg.clone());
        }
    }
    fn peer_list(&self) -> Vec<Peer> {
        self.peers
            .iter()
            .map(|(id, h)| Peer {
                id: id.clone(),
                name: h.name.clone(),
            })
            .collect()
    }
}

#[derive(Default)]
pub struct RoomRegistry {
    rooms: Mutex<HashMap<RoomId, Room>>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a peer: send it `Welcome` + the current `State`, and tell everyone
    /// else it joined.
    pub fn join(&self, room: &str, id: PeerId, name: String, tx: Tx) {
        let mut rooms = self.rooms.lock().unwrap();
        let r = rooms.entry(room.to_string()).or_default();

        let _ = tx.send(ServerMsg::Welcome {
            you: id.clone(),
            peers: r.peer_list(),
        });
        let _ = tx.send(r.state.to_state());

        r.broadcast(&ServerMsg::PeerJoined {
            peer: Peer {
                id: id.clone(),
                name: name.clone(),
            },
        });
        r.peers.insert(id, PeerHandle { name, tx });
    }

    pub fn leave(&self, room: &str, id: &str) {
        let mut rooms = self.rooms.lock().unwrap();
        if let Some(r) = rooms.get_mut(room) {
            r.peers.remove(id);
            r.broadcast(&ServerMsg::PeerLeft { id: id.to_string() });
            if r.peers.is_empty() {
                rooms.remove(room);
            }
        }
    }

    pub fn set_play(&self, room: &str, playing: bool, at: f64) {
        self.update(room, |s| {
            s.playing = playing;
            s.position = at;
            s.updated_at = now();
        });
    }

    pub fn seek(&self, room: &str, to: f64) {
        self.update(room, |s| {
            s.position = to;
            s.updated_at = now();
        });
    }

    pub fn load_video(&self, room: &str, video_id: String) {
        self.update(room, |s| {
            s.video_id = Some(video_id);
            s.position = 0.0;
            s.playing = false;
            s.updated_at = now();
        });
    }

    fn update(&self, room: &str, f: impl FnOnce(&mut Playback)) {
        let mut rooms = self.rooms.lock().unwrap();
        if let Some(r) = rooms.get_mut(room) {
            f(&mut r.state);
            let msg = r.state.to_state();
            r.broadcast(&msg);
        }
    }

    /// Relay a WebRTC signaling payload to one specific peer (Phase 3).
    pub fn relay_signal(&self, room: &str, from: PeerId, to: &str, kind: SignalKind, payload: String) {
        let rooms = self.rooms.lock().unwrap();
        if let Some(r) = rooms.get(room) {
            if let Some(h) = r.peers.get(to) {
                let _ = h.tx.send(ServerMsg::Signal { from, kind, payload });
            }
        }
    }

    /// Periodic re-broadcast so clients that drifted during playback re-align.
    pub fn heartbeat(&self) {
        let rooms = self.rooms.lock().unwrap();
        for r in rooms.values() {
            if r.state.playing {
                r.broadcast(&r.state.to_state());
            }
        }
    }
}
