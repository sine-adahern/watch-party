use leptos::*;
use shared::Video;
use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;

use crate::api;

#[component]
pub fn App() -> impl IntoView {
    let videos = create_rw_signal(Vec::<Video>::new());
    let current = create_rw_signal(None::<Video>);
    let status = create_rw_signal(String::new());

    // Load the library once on start.
    spawn_local(async move {
        match api::fetch_videos().await {
            Ok(v) => videos.set(v),
            Err(e) => status.set(format!("Could not load library: {e}")),
        }
    });

    let on_change = move |ev: web_sys::Event| {
        let input: HtmlInputElement = ev.target().unwrap().unchecked_into();
        let Some(files) = input.files() else { return };
        let Some(file) = files.get(0) else { return };
        status.set(format!("Uploading {}…", file.name()));
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
            <p class="hint">
                "Phase 1 — upload a home video and play it. Sync + video calls come next."
            </p>

            <label class="upload">
                "Upload MP4: "
                <input type="file" accept="video/mp4" on:change=on_change/>
            </label>

            <p class="status">{move || status.get()}</p>

            <Show
                when=move || current.get().is_some()
                fallback=|| view! { <p class="empty">"Pick a video from the library to start."</p> }
            >
                {move || current.get().map(|v| view! {
                    <video class="player" controls=true src=format!("/media/{}.mp4", v.id)></video>
                })}
            </Show>

            <h2>"Library"</h2>
            <ul class="library">
                <For each=move || videos.get() key=|v| v.id.clone() let:v>
                    <li>
                        <button on:click={
                            let picked = v.clone();
                            move |_| current.set(Some(picked.clone()))
                        }>
                            {v.name.clone()}
                        </button>
                    </li>
                </For>
            </ul>
        </main>
    }
}
