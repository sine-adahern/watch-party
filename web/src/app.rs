//! Phase 3 frontend: sync (Phase 2) plus a WebRTC video call.
//!
//! On Join we open the WebSocket, grab camera/mic, and send `Join`. Peer and
//! signal messages drive the mesh in `crate::call`; `State` messages keep the
//! shared movie `<video>` in sync. Server sync + relay are verified end-to-end;
//! this WASM module builds with `trunk` on your toolchain.

use leptos::html::{Div, Video};
use leptos::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlInputElement, HtmlVideoElement, MessageEvent, WebSocket};

use shared::{ClientMsg, Peer, ServerMsg, SignalKind, Video as VideoMeta};

use crate::api;
use crate::call::Call;

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn send(ws: &WebSocket, msg: &ClientMsg) {
    if let Ok(json) = serde_json::to_string(msg) {
        let _ = ws.send_with_str(&json);
    }
}

#[component]
pub fn App() -> impl IntoView {
    let videos = create_rw_signal(Vec::<VideoMeta>::new());
    let current = create_rw_signal(None::<VideoMeta>);
    let status = create_rw_signal(String::new());
    let peers = create_rw_signal(Vec::<Peer>::new());
    let me = create_rw_signal(None::<String>);
    let joined = create_rw_signal(false);
    let room_input = create_rw_signal("main".to_string());
    let name_input = create_rw_signal(String::new());
    let suppress_until = create_rw_signal(0.0_f64);
    let ws_sig = create_rw_signal(None::<WebSocket>);
    let call_sig = create_rw_signal(None::<Call>);

    let video_ref = create_node_ref::<Video>(); // the shared movie
    let local_video_ref = create_node_ref::<Video>(); // your camera
    let remote_ref = create_node_ref::<Div>(); // container for others' cameras

    spawn_local(async move {
        match api::fetch_videos().await {
            Ok(v) => videos.set(v),
            Err(e) => status.set(format!("Could not load library: {e}")),
        }
    });

    // Apply an authoritative playback State to the shared movie element.
    let apply_state = move |playing: bool, position: f64, video_id: Option<String>| {
        if let Some(vid) = &video_id {
            let need = current.get_untracked().map(|c| &c.id != vid).unwrap_or(true);
            if need {
                if let Some(v) = videos.get_untracked().into_iter().find(|v| &v.id == vid) {
                    current.set(Some(v));
                }
            }
        }
        suppress_until.set(now_ms() + 700.0);
        if let Some(el) = video_ref.get_untracked() {
            if playing {
                let drift = el.current_time() - position;
                if drift.abs() > 0.3 {
                    el.set_current_time(position);
                }
                let _ = el.play();
            } else {
                let _ = el.pause();
                el.set_current_time(position);
            }
        }
    };

    let do_join = move |_| {
        if joined.get_untracked() {
            return;
        }
        let room = room_input.get_untracked();
        let name = {
            let n = name_input.get_untracked();
            if n.trim().is_empty() { "guest".to_string() } else { n }
        };

        // Grab the (always-rendered) call elements for the mesh.
        let (Some(lv), Some(rc)) = (local_video_ref.get_untracked(), remote_ref.get_untracked())
        else {
            status.set("Video area not ready yet — try again.".into());
            return;
        };
        let local_video: HtmlVideoElement = (*lv).clone();
        let remote_container: Element = (*rc).clone().unchecked_into();

        let loc = web_sys::window().unwrap().location();
        let scheme = if loc.protocol().unwrap_or_default() == "https:" { "wss" } else { "ws" };
        let host = loc.host().unwrap_or_default();
        let Ok(ws) = WebSocket::new(&format!("{scheme}://{host}/ws")) else {
            status.set("Could not open WebSocket".into());
            return;
        };

        let call = Call::new(ws.clone(), remote_container, local_video);
        call_sig.set(Some(call.clone()));

        // On open: get media first, then announce Join (so we can answer with
        // our tracks already attached).
        {
            let call = call.clone();
            let ws2 = ws.clone();
            let room2 = room.clone();
            let name2 = name.clone();
            let vid = current.get_untracked().map(|v| v.id);
            let onopen = Closure::<dyn FnMut()>::new(move || {
                let call = call.clone();
                let ws2 = ws2.clone();
                let room2 = room2.clone();
                let name2 = name2.clone();
                let vid = vid.clone();
                spawn_local(async move {
                    call.start_local_media().await;
                    send(&ws2, &ClientMsg::Join { room: room2, name: name2, video_id: vid });
                });
            });
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            onopen.forget();
        }

        // Route incoming server messages.
        {
            let call = call.clone();
            let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                let Some(txt) = e.data().as_string() else { return };
                let Ok(msg) = serde_json::from_str::<ServerMsg>(&txt) else { return };
                match msg {
                    ServerMsg::Welcome { you, peers: list } => {
                        me.set(Some(you));
                        peers.set(list);
                    }
                    ServerMsg::PeerJoined { peer } => {
                        let id = peer.id.clone();
                        peers.update(|ps| {
                            if !ps.iter().any(|p| p.id == peer.id) {
                                ps.push(peer);
                            }
                        });
                        call.on_peer_joined(&id); // we're already here -> we offer
                    }
                    ServerMsg::PeerLeft { id } => {
                        peers.update(|ps| ps.retain(|p| p.id != id));
                        call.on_peer_left(&id);
                    }
                    ServerMsg::State { playing, position, video_id, .. } => {
                        apply_state(playing, position, video_id);
                    }
                    ServerMsg::Signal { from, kind, payload } => match kind {
                        SignalKind::Offer => call.on_offer(&from, payload),
                        SignalKind::Answer => call.on_answer(&from, payload),
                        SignalKind::IceCandidate => call.on_ice(&from, payload),
                    },
                }
            });
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();
        }

        ws_sig.set(Some(ws));
        joined.set(true);
        status.set(format!("Joined room \u{201c}{room}\u{201d}"));
    };

    // User video events -> control messages (ignored during the suppress window).
    let guarded_send = move |make: &dyn Fn(f64) -> ClientMsg| {
        if now_ms() < suppress_until.get_untracked() {
            return;
        }
        if let (Some(ws), Some(el)) = (ws_sig.get_untracked(), video_ref.get_untracked()) {
            send(&ws, &make(el.current_time()));
        }
    };
    let on_play = move |_| guarded_send(&|at| ClientMsg::Play { at });
    let on_pause = move |_| guarded_send(&|at| ClientMsg::Pause { at });
    let on_seeked = move |_| guarded_send(&|to| ClientMsg::Seek { to });

    let on_change = move |ev: web_sys::Event| {
        let input: HtmlInputElement = ev.target().unwrap().unchecked_into();
        let Some(files) = input.files() else { return };
        let Some(file) = files.get(0) else { return };
        status.set(format!("Uploading {}\u{2026}", file.name()));
        spawn_local(async move {
            match api::upload(file).await {
                Ok(v) => {
                    status.set(format!("Uploaded {}", v.name));
                    if let Ok(list) = api::fetch_videos().await {
                        videos.set(list);
                    }
                }
                Err(e) => status.set(format!("Upload failed: {e}")),
            }
        });
    };

    view! {
        <main class="wrap">
            <h1>"Watch Party"</h1>

            <Show
                when=move || !joined.get()
                fallback=move || view! {
                    <p class="room-bar">{move || format!(
                        "In room \u{201c}{}\u{201d} \u{00b7} {} watching",
                        room_input.get(), peers.get().len().max(1),
                    )}</p>
                }
            >
                <div class="join">
                    <input class="inp" placeholder="room code"
                        prop:value=move || room_input.get()
                        on:input=move |e| room_input.set(event_target_value(&e))/>
                    <input class="inp" placeholder="your name"
                        prop:value=move || name_input.get()
                        on:input=move |e| name_input.set(event_target_value(&e))/>
                    <button on:click=do_join>"Join"</button>
                </div>
            </Show>

            <p class="status">{move || status.get()}</p>

            // The video call. Always rendered so the node refs exist at Join time.
            <div class="call">
                <video node_ref=local_video_ref class="me"></video>
                <div node_ref=remote_ref class="remotes"></div>
            </div>

            <Show
                when=move || current.get().is_some()
                fallback=|| view! { <p class="empty">"Pick a video from the library to start."</p> }
            >
                {move || current.get().map(|v| view! {
                    <video
                        node_ref=video_ref
                        class="player"
                        controls=true
                        src=format!("/media/{}.mp4", v.id)
                        on:play=on_play
                        on:pause=on_pause
                        on:seeked=on_seeked
                    ></video>
                })}
            </Show>

            <label class="upload">"Upload MP4: "
                <input type="file" accept="video/mp4" on:change=on_change/>
            </label>

            <h2>"Library"</h2>
            <ul class="library">
                <For each=move || videos.get() key=|v| v.id.clone() let:v>
                    <li>
                        <button on:click={
                            let v = v.clone();
                            move |_| {
                                current.set(Some(v.clone()));
                                if let Some(ws) = ws_sig.get_untracked() {
                                    send(&ws, &ClientMsg::LoadVideo { video_id: v.id.clone() });
                                }
                            }
                        }>{v.name.clone()}</button>
                    </li>
                </For>
            </ul>

            {move || joined.get().then(|| view! {
                <h2>"Watching now"</h2>
                <ul class="peers">
                    <For each=move || peers.get() key=|p| p.id.clone() let:p>
                        <li>{
                            let mine = me.get_untracked().as_deref() == Some(p.id.as_str());
                            if mine { format!("{} (you)", p.name) } else { p.name.clone() }
                        }</li>
                    </For>
                </ul>
            })}
        </main>
    }
}
