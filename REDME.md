
# Watch Party — Rust scaffold (Phase 1)

A synchronized home-video watch party. Upload MP4s you own, watch them together
in the browser, with playback controls that stay in sync and a video call
alongside. Backend is Rust (Axum); frontend is Rust compiled to WASM (Leptos).
No SFU — with ≤4 people the video call is a plain WebRTC mesh.

**This scaffold is Phase 1:** upload a video, list the library, play one in the
browser with working seek. The sync layer and calls are stubbed for the next
phases (the `shared` protocol and the `/ws` endpoint already exist).

## What's verified

The `shared` and `server` crates compile and were smoke-tested: upload → list →
range/seek all work (`GET /media/<id>.mp4` answers `206 Partial Content`). The
`web` crate is written against Leptos 0.6 and builds with `trunk` on a recent
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
│     ├─ media.rs        upload + range serving + JSON library
│     ├─ ws.rs           WebSocket (Phase 1: echo; Phase 2: sync)
│     └─ rooms.rs        room registry (Phase 2 seam)
└─ web/                  Leptos frontend → WASM
   ├─ index.html         trunk entry + styles
   ├─ Trunk.toml         dev proxy to the backend
   └─ src/
      ├─ main.rs         mount
      ├─ app.rs          UI: upload, library, player
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

Open http://localhost:8080, upload an MP4, click it in the library, and it
plays. Uploaded files land in `./media/` with a `videos.json` index.

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
| GET    | `/ws`             | WebSocket (echo now; sync next)           |

## Next phases

- **Phase 2 — sync.** Replace `ws::handle_socket` with room join + authoritative
  playback state. On any `ClientMsg::{Play,Pause,Seek}`, update the room's state,
  stamp it with the server clock, and broadcast `ServerMsg::State` to everyone.
  Clients reconcile: expected = `position + (now - server_time)` while playing;
  nudge the `<video>` rate by ±5% for small drift instead of hard-seeking.
- **Phase 3 — calls.** WebRTC mesh (≤4 peers). Relay SDP/ICE via the existing
  `/ws` using `ClientMsg::Signal` / `ServerMsg::Signal`. Add public STUN, then a
  self-hosted `coturn` for NAT fallback.
- **Phase 4 — polish.** Room codes/invite links, presence list, reconnect,
  `ffmpeg -movflags +faststart` on upload, swap the JSON index for SQLite
  (`sqlx`), deploy on one small VPS running the server + coturn.

## Notes

- Dependency versions are pinned conservatively; bump to Axum 0.8 / Leptos 0.7
  whenever you like.
- Uploads stream to disk with a 4 GiB cap (`MAX_UPLOAD` in `server/src/media.rs`).
- Set `MEDIA_DIR` to change where videos are stored (default `./media`).
