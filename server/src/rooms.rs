//! Room registry — a Phase 2 seam.
//!
//! Each watch party will become one entry here owning the authoritative
//! playback state and a `tokio::sync::broadcast` channel that fans
//! [`shared::ServerMsg`] out to every connected client in the room. For Phase 1
//! it's an empty shell so the wiring in `main.rs` already exists.

use std::collections::HashMap;

use tokio::sync::Mutex;

#[derive(Default)]
pub struct RoomRegistry {
    #[allow(dead_code)]
    rooms: Mutex<HashMap<shared::RoomId, ()>>,
}

impl RoomRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}
