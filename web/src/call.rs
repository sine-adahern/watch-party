//! Phase 3: a WebRTC mesh for up to ~4 people.
//!
//! Each remote peer gets one `RtcPeerConnection`. Connection setup is driven by
//! the server's peer events and the `Signal` relay:
//!   * an *existing* peer initiates the offer when a newcomer joins
//!     (`on_peer_joined`), so each pair has exactly one initiator (no glare);
//!   * the newcomer answers incoming offers (`on_offer`);
//!   * ICE candidates are exchanged as they're gathered.
//!
//! JS config objects are built with `Reflect` to stay robust across web-sys
//! versions. This module is unverified in the sandbox (no WASM target); it is
//! written against the standard web-sys WebRTC API and builds with `trunk`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use js_sys::{Array, Object, Reflect, JSON};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    Element, HtmlVideoElement, MediaStream, MediaStreamConstraints, MediaStreamTrack,
    RtcConfiguration, RtcIceCandidateInit, RtcPeerConnection, RtcPeerConnectionIceEvent,
    RtcSessionDescriptionInit, RtcTrackEvent,
};

use shared::{ClientMsg, SignalKind};

const STUN: &str = "stun:stun.l.google.com:19302";

#[derive(Clone)]
pub struct Call(Rc<Inner>);

struct Inner {
    ws: web_sys::WebSocket,
    local: RefCell<Option<MediaStream>>,
    peers: RefCell<HashMap<String, RtcPeerConnection>>,
    remote_container: Element,
    local_video: HtmlVideoElement,
}

impl Inner {
    fn send_signal(&self, to: &str, kind: SignalKind, payload: String) {
        let msg = ClientMsg::Signal {
            to: to.to_string(),
            kind,
            payload,
        };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = self.ws.send_with_str(&json);
        }
    }
}

impl Call {
    pub fn new(ws: web_sys::WebSocket, remote_container: Element, local_video: HtmlVideoElement) -> Self {
        Call(Rc::new(Inner {
            ws,
            local: RefCell::new(None),
            peers: RefCell::new(HashMap::new()),
            remote_container,
            local_video,
        }))
    }

    /// Ask for camera + mic and show the local self-view (muted).
    pub async fn start_local_media(&self) {
        let Some(nav) = web_sys::window().map(|w| w.navigator()) else { return };
        let Ok(devices) = nav.media_devices() else { return };

        let c = Object::new();
        let _ = Reflect::set(&c, &"audio".into(), &JsValue::TRUE);
        let _ = Reflect::set(&c, &"video".into(), &JsValue::TRUE);
        let constraints: MediaStreamConstraints = c.unchecked_into();

        let Ok(promise) = devices.get_user_media_with_constraints(&constraints) else { return };
        if let Ok(stream) = JsFuture::from(promise).await {
            let stream: MediaStream = stream.unchecked_into();
            self.0.local_video.set_muted(true);
            self.0.local_video.set_autoplay(true);
            let _ = self.0.local_video.set_attribute("playsinline", "");
            self.0.local_video.set_src_object(Some(&stream));
            let _ = self.0.local_video.play();
            *self.0.local.borrow_mut() = Some(stream);
        }
    }

    fn create_pc(&self, pid: &str) -> RtcPeerConnection {
        // { iceServers: [{ urls: STUN }] }
        let server = Object::new();
        let _ = Reflect::set(&server, &"urls".into(), &STUN.into());
        let ice = Array::new();
        ice.push(&server);
        let cfg = Object::new();
        let _ = Reflect::set(&cfg, &"iceServers".into(), &ice);
        let config: RtcConfiguration = cfg.unchecked_into();

        let pc = RtcPeerConnection::new_with_configuration(&config)
            .expect("RtcPeerConnection");

        // Add our outgoing tracks.
        if let Some(stream) = self.0.local.borrow().as_ref() {
            let tracks = stream.get_tracks();
            for i in 0..tracks.length() {
                let track: MediaStreamTrack = tracks.get(i).unchecked_into();
                let _ = pc.add_track(&track, stream);
            }
        }

        // Send our ICE candidates as they're found.
        {
            let this = self.clone();
            let pid = pid.to_string();
            let cb = Closure::<dyn FnMut(RtcPeerConnectionIceEvent)>::new(
                move |ev: RtcPeerConnectionIceEvent| {
                    if let Some(cand) = ev.candidate() {
                        let json = JSON::stringify(&cand.to_json())
                            .ok()
                            .and_then(|s| s.as_string())
                            .unwrap_or_default();
                        this.0.send_signal(&pid, SignalKind::IceCandidate, json);
                    }
                },
            );
            pc.set_onicecandidate(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        }

        // Show incoming media.
        {
            let this = self.clone();
            let pid = pid.to_string();
            let cb = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |ev: RtcTrackEvent| {
                let streams = ev.streams();
                if streams.length() > 0 {
                    let stream: MediaStream = streams.get(0).unchecked_into();
                    this.attach_remote(&pid, &stream);
                }
            });
            pc.set_ontrack(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        }

        self.0.peers.borrow_mut().insert(pid.to_string(), pc.clone());
        pc
    }

    fn get_or_create(&self, pid: &str) -> RtcPeerConnection {
        let existing = self.0.peers.borrow().get(pid).cloned();
        existing.unwrap_or_else(|| self.create_pc(pid))
    }

    fn attach_remote(&self, pid: &str, stream: &MediaStream) {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        let id = format!("remote-{pid}");
        if doc.get_element_by_id(&id).is_none() {
            if let Ok(el) = doc.create_element("video") {
                let video: HtmlVideoElement = el.unchecked_into();
                video.set_id(&id);
                video.set_autoplay(true);
                video.set_class_name("remote");
                let _ = video.set_attribute("playsinline", "");
                let _ = self.0.remote_container.append_child(&video);
            }
        }
        if let Some(el) = doc.get_element_by_id(&id) {
            let video: HtmlVideoElement = el.unchecked_into();
            video.set_src_object(Some(stream));
            let _ = video.play();
        }
    }

    fn remove_remote(&self, pid: &str) {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(el) = doc.get_element_by_id(&format!("remote-{pid}")) {
                el.remove();
            }
        }
    }

    /// We were already here; `pid` just joined -> we make the offer.
    pub fn on_peer_joined(&self, pid: &str) {
        let this = self.clone();
        let pid = pid.to_string();
        spawn_local(async move {
            let pc = this.create_pc(&pid);
            if let Ok(offer) = JsFuture::from(pc.create_offer()).await {
                let offer: RtcSessionDescriptionInit = offer.unchecked_into();
                let _ = JsFuture::from(pc.set_local_description(&offer)).await;
                let sdp = sdp_of(&offer);
                this.0.send_signal(&pid, SignalKind::Offer, sdp);
            }
        });
    }

    pub fn on_offer(&self, from: &str, sdp: String) {
        let this = self.clone();
        let from = from.to_string();
        spawn_local(async move {
            let pc = this.get_or_create(&from);
            let _ = JsFuture::from(pc.set_remote_description(&make_desc("offer", &sdp))).await;
            if let Ok(answer) = JsFuture::from(pc.create_answer()).await {
                let answer: RtcSessionDescriptionInit = answer.unchecked_into();
                let _ = JsFuture::from(pc.set_local_description(&answer)).await;
                this.0.send_signal(&from, SignalKind::Answer, sdp_of(&answer));
            }
        });
    }

    pub fn on_answer(&self, from: &str, sdp: String) {
        if let Some(pc) = self.0.peers.borrow().get(from).cloned() {
            spawn_local(async move {
                let _ = JsFuture::from(pc.set_remote_description(&make_desc("answer", &sdp))).await;
            });
        }
    }

    pub fn on_ice(&self, from: &str, payload: String) {
        if let Some(pc) = self.0.peers.borrow().get(from).cloned() {
            if let Ok(obj) = JSON::parse(&payload) {
                let init: RtcIceCandidateInit = obj.unchecked_into();
                let _ = pc.add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(&init));
            }
        }
    }

    pub fn on_peer_left(&self, pid: &str) {
        if let Some(pc) = self.0.peers.borrow_mut().remove(pid) {
            pc.close();
        }
        self.remove_remote(pid);
    }
}

fn make_desc(kind: &str, sdp: &str) -> RtcSessionDescriptionInit {
    let o = Object::new();
    let _ = Reflect::set(&o, &"type".into(), &kind.into());
    let _ = Reflect::set(&o, &"sdp".into(), &sdp.into());
    o.unchecked_into()
}

fn sdp_of(desc: &RtcSessionDescriptionInit) -> String {
    Reflect::get(desc, &"sdp".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}
