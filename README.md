# Watch Party — Rust scaffold (Phase 1)

A synchronized home-video watch party. Upload MP4s you own, watch them together
in the browser, with playback controls that stay in sync and a video call
alongside. Backend is Rust (Axum); frontend is Rust compiled to WASM (Leptos).
No SFU — with ≤4 people the video call is a plain WebRTC mesh.

**This scaffold covers Phases 1–4:** upload/list/play home videos, in-room
playback sync (pause/play/seek/skip + load), a WebRTC video call (mesh, ~4
people), plus polish — SQLite-backed library, delete, optional `ffmpeg` faststart
on upload, invite links (`?room=CODE`), and WebSocket auto-reconnect. Ships with a
multi-stage `Dockerfile` and an example `coturn` config.

## What's verified

The `shared` and `server` crates compile and were tested end-to-end:
- upload → list → range/seek (`GET /media/<id>.mp4` → `206 Partial Content`);
- sync over `/ws` with two live clients — join, play, seek, pause, load-video,
  peer join/leave, and the heartbeat re-broadcast all propagate correctly;
- signal relay — offer, answer, and ICE candidates route peer-to-peer with the
  correct `from` attribution;
- persistence — uploads survive a server restart (SQLite), delete removes row +
  file, and faststart degrades cleanly when `ffmpeg` isn't installed.

The `web` crate is written against Leptos 0.6 and builds with `trunk` on a recent
stable Rust toolchain (edition-2024 dependencies mean you'll want Rust ≥ 1.85).

## Layout

```
watchparty/
├─ Cargo.toml            workspace (builds shared + server natively)
├─ rust-toolchain.toml   stable + wasm32 target
├─ shared/               protocol types shared by server AND frontend
│  └─ src/lib.rs         Video, ClientMsg, ServerMsg, ...
├─ server/               Axum backend
│  └─ src/
│     ├─ main.rs         router + app state
│     ├─ media.rs        upload + range serving + delete + faststart
│     ├─ db.rs           SQLite library (rusqlite, bundled)
│     ├─ ws.rs           WebSocket: parse ClientMsg, drive room, stream ServerMsg
│     └─ rooms.rs        rooms: authoritative playback state + fan-out
├─ Dockerfile           multi-stage build (web + server) -> slim runtime
├─ deploy/coturn.conf    example TURN config
└─ web/                  Leptos frontend → WASM
   ├─ index.html         trunk entry + styles
   ├─ Trunk.toml         dev proxy to the backend
   └─ src/
      ├─ main.rs         mount
      ├─ app.rs          UI: join, synced player, call tiles, peers, library
      └─ api.rs          fetch helpers
```

## Prerequisites

```bash
# Rust (recent stable), then:
rustup target add wasm32-unknown-unknown
cargo install trunk
```

## Run it (development)

Two terminals from the repo root:

```bash
# 1) backend on :3000
cargo run -p server

# 2) frontend on :8080 (proxies /api, /media, /ws to :3000)
cd web && trunk serve
```

Open http://localhost:8080 in two browser windows. Enter the same room code in
both and click Join (allow camera/mic when prompted — you'll see each other in
the tiles). Pick a video from the library and press play in one — the other
follows; pause, seek, and switching videos all stay in sync. Uploaded files land
in `./media/` with a `videos.json` index.

Camera/mic (`getUserMedia`) needs a secure context. `http://localhost` counts as
secure, so same-machine testing works over plain HTTP. To test across *different*
devices you'll need HTTPS (a reverse proxy with a cert, or a tunnel like
`cloudflared`/`ngrok`).

## Run it (production, single binary + static assets)

```bash
cd web && trunk build --release      # outputs web/dist
cd .. && cargo run -p server --release
# Axum serves the built frontend AND the API/media on http://localhost:3000
```

## Endpoints

| Method | Path              | Purpose                                   |
|--------|-------------------|-------------------------------------------|
| GET    | `/api/videos`     | JSON list of uploaded videos              |
| POST   | `/api/upload`     | raw MP4 body + `x-filename` header        |
| GET    | `/media/<id>.mp4` | video stream, supports Range (seek)       |
| DELETE | `/api/videos/<id>`| remove a video (row + file)               |
| GET    | `/ws`             | WebSocket: sync + WebRTC signaling relay  |

## Next phases

- **Phase 2 — sync. (done)** Room actors hold authoritative playback state; any
  `Play/Pause/Seek/LoadVideo` restamps it with the server clock and broadcasts
  `ServerMsg::State`. Clients hard-correct when drift > 0.3s; a 3s heartbeat
  re-aligns anyone who fell behind. (A gentle ±5% playbackRate nudge instead of
  a hard seek is an easy refinement in `apply_state`.)
- **Phase 3 — calls. (done)** A WebRTC mesh for up to ~4 peers in `web/src/call.rs`.
  The peer already in the room offers to each newcomer (one initiator per pair, no
  glare); SDP/ICE flow through `Signal`. Uses public STUN only right now — add a
  self-hosted **coturn** (TURN) for anyone behind a strict NAT, and put its
  credentials in the `iceServers` config in `create_pc`.
- **Phase 4 — polish.** Room codes/invite links, presence list, reconnect,
  `ffmpeg -movflags +faststart` on upload, swap the JSON index for SQLite
  (`sqlx`), deploy on one small VPS running the server + coturn.

## Notes

- Dependency versions are pinned conservatively; bump to Axum 0.8 / Leptos 0.7
  whenever you like.
- The call uses public STUN only; add TURN (coturn) before relying on it across
  arbitrary networks.
- Uploads stream to disk with a 4 GiB cap (`MAX_UPLOAD` in `server/src/media.rs`).
- Set `MEDIA_DIR` to change where videos are stored (default `./media`).
