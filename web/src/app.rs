//! Phase 4 frontend: everything from Phases 1–3 plus polish —
//! invite links (`?room=CODE`), a Copy-invite button, per-video delete, and
//! WebSocket auto-reconnect. The current socket lives in a shared cell so the
//! call keeps signaling across reconnects.
//!
//! Server sync/relay/persistence are verified end-to-end; this WASM module is
//! written against Leptos 0.6 and builds with `trunk` on your toolchain.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::html::{Div, Video};
use leptos::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlInputElement, HtmlVideoElement, MessageEvent, WebSocket};

use shared::{ClientMsg, Peer, ServerMsg, SignalKind, Video as VideoMeta};

use crate::api;
use crate::call::Call;

/// The current WebSocket, shared between the app and the call session.
type Sock = Rc<RefCell<Option<WebSocket>>>;

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn send_via(sock: &Sock, msg: &ClientMsg) {
    if let (Some(ws), Ok(json)) = (sock.borrow().as_ref(), serde_json::to_string(msg)) {
        let _ = ws.send_with_str(&json);
    }
}

/// Read `?room=CODE` from the URL, for invite links.
fn url_room() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get("room").filter(|s| !s.is_empty())
}

#[component]
pub fn App() -> impl IntoView {
    let videos = create_rw_signal(Vec::<VideoMeta>::new());
    let current = create_rw_signal(None::<VideoMeta>);
    let status = create_rw_signal(String::new());
    let peers = create_rw_signal(Vec::<Peer>::new());
    let me = create_rw_signal(None::<String>);
    let joined = create_rw_signal(false);
    let room_input = create_rw_signal(url_room().unwrap_or_else(|| "main".to_string()));
    let name_input = create_rw_signal(String::new());
    let suppress_until = create_rw_signal(0.0_f64);

    let sock: Sock = Rc::new(RefCell::new(None));
    let call_sig = create_rw_signal(None::<Call>);

    let video_ref = create_node_ref::<Video>();
    let local_video_ref = create_node_ref::<Video>();
    let remote_ref = create_node_ref::<Div>();

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

    // (Re)connect the socket and wire handlers. Stored in a slot so `onclose`
    // can re-invoke it for auto-reconnect.
    let connect_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let connect: Rc<dyn Fn()> = {
        let sock = sock.clone();
        let slot = connect_slot.clone();
        Rc::new(move || {
            let Some(call) = call_sig.get_untracked() else { return };
            call.reset(); // clear any stale peer connections before (re)joining

            let loc = web_sys::window().unwrap().location();
            let scheme = if loc.protocol().unwrap_or_default() == "https:" { "wss" } else { "ws" };
            let host = loc.host().unwrap_or_default();
            let Ok(ws) = WebSocket::new(&format!("{scheme}://{host}/ws")) else {
                status.set("Could not open WebSocket".into());
                return;
            };

            // On open: announce Join (media was started in do_join).
            {
                let ws2 = ws.clone();
                let onopen = Closure::<dyn FnMut()>::new(move || {
                    let name = {
                        let n = name_input.get_untracked();
                        if n.trim().is_empty() { "guest".to_string() } else { n }
                    };
                    let msg = ClientMsg::Join {
                        room: room_input.get_untracked(),
                        name,
                        video_id: current.get_untracked().map(|v| v.id),
                    };
                    if let Ok(j) = serde_json::to_string(&msg) {
                        let _ = ws2.send_with_str(&j);
                    }
                    status.set("Connected".into());
                });
                ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
                onopen.forget();
            }

            // Route server messages.
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
                            call.on_peer_joined(&id);
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

            // On close: reconnect shortly, if we're still meant to be joined.
            {
                let slot = slot.clone();
                let onclose = Closure::<dyn FnMut()>::new(move || {
                    if joined.get_untracked() {
                        status.set("Reconnecting\u{2026}".into());
                        let slot = slot.clone();
                        gloo_timers::callback::Timeout::new(1000, move || {
                            if let Some(f) = slot.borrow().clone() {
                                f();
                            }
                        })
                        .forget();
                    }
                });
                ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
                onclose.forget();
            }

            *sock.borrow_mut() = Some(ws);
        })
    };
    *connect_slot.borrow_mut() = Some(connect.clone());

    // Join: create the call + start media once, then connect.
    let do_join = {
        let sock = sock.clone();
        let connect = connect.clone();
        move |_| {
            if joined.get_untracked() {
                return;
            }
            let (Some(lv), Some(rc)) =
                (local_video_ref.get_untracked(), remote_ref.get_untracked())
            else {
                status.set("Video area not ready yet — try again.".into());
                return;
            };
            let local_video: HtmlVideoElement = (*lv).clone();
            let remote_container: Element = (*rc).clone().unchecked_into();

            let call = Call::new(sock.clone(), remote_container, local_video);
            call_sig.set(Some(call.clone()));

            let connect = connect.clone();
            spawn_local(async move {
                call.start_local_media().await;
                connect();
            });

            joined.set(true);
            status.set(format!("Joining \u{201c}{}\u{201d}\u{2026}", room_input.get_untracked()));
        }
    };

    let copy_invite = move |_| {
        if let Some(win) = web_sys::window() {
            let origin = win.location().origin().unwrap_or_default();
            let link = format!("{origin}/?room={}", room_input.get_untracked());
            let _ = win.navigator().clipboard().write_text(&link);
            status.set("Invite link copied".into());
        }
    };

    // User video events -> control messages (ignored during the suppress window).
    let on_play = {
        let sock = sock.clone();
        move |_| {
            if now_ms() >= suppress_until.get_untracked() {
                if let Some(el) = video_ref.get_untracked() {
                    send_via(&sock, &ClientMsg::Play { at: el.current_time() });
                }
            }
        }
    };
    let on_pause = {
        let sock = sock.clone();
        move |_| {
            if now_ms() >= suppress_until.get_untracked() {
                if let Some(el) = video_ref.get_untracked() {
                    send_via(&sock, &ClientMsg::Pause { at: el.current_time() });
                }
            }
        }
    };
    let on_seeked = {
        let sock = sock.clone();
        move |_| {
            if now_ms() >= suppress_until.get_untracked() {
                if let Some(el) = video_ref.get_untracked() {
                    send_via(&sock, &ClientMsg::Seek { to: el.current_time() });
                }
            }
        }
    };

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

    let library_sock = sock.clone();

    view! {
        <main class="wrap">
            <h1>"Watch Party"</h1>

            <Show
                when=move || !joined.get()
                fallback=move || view! {
                    <div class="room-bar">
                        <span>{move || format!(
                            "In room \u{201c}{}\u{201d} \u{00b7} {} watching",
                            room_input.get(), peers.get().len().max(1),
                        )}</span>
                        <button class="link" on:click=copy_invite>"Copy invite link"</button>
                    </div>
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
                        <button class="pick" on:click={
                            let sock = library_sock.clone();
                            let v = v.clone();
                            move |_| {
                                current.set(Some(v.clone()));
                                send_via(&sock, &ClientMsg::LoadVideo { video_id: v.id.clone() });
                            }
                        }>{v.name.clone()}</button>
                        <button class="del" title="Delete" on:click={
                            let vid = v.id.clone();
                            move |_| {
                                let vid = vid.clone();
                                spawn_local(async move {
                                    if api::delete_video(&vid).await.is_ok() {
                                        if let Ok(list) = api::fetch_videos().await {
                                            videos.set(list);
                                        }
                                    }
                                });
                            }
                        }>"\u{00d7}"</button>
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
