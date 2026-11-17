//! deadwax — the controller for the household playhead.
//!
//! The Spore owns the view and the interaction; the host owns the audio device,
//! the queue writer, the Spotify session, and the Unibus client. Transport goes
//! through `soil:playback`; the owned library comes from `deadwax serve`
//! through one bounded `soil:http` GET. Nothing here sees a path, a token, a
//! peer, or a PCM sample: a local queue entry is a CID, a Spotify entry is a
//! URI, and the owner resolves both.
//!
//! The control plane is Unibus: `audio.playback.state.v1` and
//! `audio.playback.event.v1` arrive here as `soil:playback/events`, and
//! `audio.playback.command.v1` leaves as `player.send`. The host bridges the two
//! without the guest opening a socket.

wit_bindgen::generate!({
    path: "wit",
    world: "runner",
    with: {
        "wasi:webgpu/webgpu@0.3.0-rc.2": wgpu_wasi_webgpu::wasi::webgpu::webgpu,
    },
    generate_all,
});

use gpui::{
    AnyWindowHandle, AppContext as _, HeadlessAppContext, Image, ImageFormat, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, PlatformTextSystem, point, px, size,
};
use gpui_component::slider::{SliderEvent, SliderState};
use gpui_wgpu::{CosmicTextSystem, WgpuContext, WgpuHeadlessRenderer};
use soil::http::http;
use soil::present::{input, surface};
use std::{borrow::Cow, sync::Arc};
pub mod view;

use view::{Deck, Request, Side, Snapshot, Track};

const LILEX: &[u8] = include_bytes!("../assets/Lilex-Regular.ttf");
const LILEX_BOLD: &[u8] = include_bytes!("../assets/Lilex-Bold.ttf");

/// The catalog API. One origin, named once: a Spore names its origins rather
/// than accepting one from whoever launched it.
const CATALOG: &str = "http://127.0.0.1:6601";
const MAX_BODY: u32 = 1024 * 1024;
const TIMEOUT_MS: u32 = 3_000;
const PAGE: u32 = 60;

fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn fetch_catalog(query: &str) -> Result<Vec<Track>, String> {
    let url = if query.trim().is_empty() {
        format!("{CATALOG}/tracks?limit={PAGE}")
    } else {
        format!("{CATALOG}/search?q={}&limit={PAGE}", urlencode(query))
    };
    let response = http::fetch(&http::Request {
        url,
        method: http::Method::Get,
        headers: vec![http::Header {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: TIMEOUT_MS,
    })
    .map_err(|error| format!("catalog: {error:?}"))?;
    if response.status != 200 {
        return Err(format!("catalog: HTTP {}", response.status));
    }
    view::parse_catalog(&response.body)
}

/// The imported playlists, with coverage: what each one is worth playing and
/// what it is missing. Same origin as the catalog, one bounded GET.
fn fetch_playlists() -> Result<Vec<view::Playlist>, String> {
    let response = http::fetch(&http::Request {
        url: format!("{CATALOG}/playlists?coverage=1"),
        method: http::Method::Get,
        headers: vec![http::Header {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: TIMEOUT_MS,
    })
    .map_err(|error| format!("playlists: {error:?}"))?;
    if response.status != 200 {
        return Err(format!("playlists: HTTP {}", response.status));
    }
    view::parse_playlists(&response.body)
}

/// The wishlist: records wanted, not records missing from a playlist.
fn fetch_wishlist() -> Result<Vec<view::Wish>, String> {
    let value = http_json(http::Method::Get, "/wishlist", None)?;
    let body = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    view::parse_wishlist(&body)
}

/// Play a playlist: the server owns the join (owned file or stream shim), so
/// the Spore asks for the queue rather than composing it itself.
fn play_playlist(owner: &str, name: &str) -> Result<(), String> {
    let response = http::fetch(&http::Request {
        url: format!(
            "{CATALOG}/queue/playlist?user={}&name={}",
            urlencode(owner),
            urlencode(name)
        ),
        method: http::Method::Post,
        headers: vec![],
        body: None,
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: TIMEOUT_MS,
    })
    .map_err(|error| format!("play playlist: {error:?}"))?;
    if response.status != 200 {
        return Err(format!("play playlist: HTTP {}", response.status));
    }
    Ok(())
}

/// The cover for a local track, by content address: the Spore knows a CID, not a
/// path, so the catalog is what joins them.
fn fetch_art(cid: &str) -> Option<Image> {
    let response = http::fetch(&http::Request {
        url: format!("{CATALOG}/mpd/art?cid={}", urlencode(cid)),
        method: http::Method::Get,
        headers: vec![],
        body: None,
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: TIMEOUT_MS,
    })
    .ok()?;
    if response.status != 200 {
        return None;
    }
    let bytes = response.body;
    let format = if bytes.starts_with(&[0xFF, 0xD8]) {
        ImageFormat::Jpeg
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else {
        return None;
    };
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    Some(Image {
        format,
        bytes,
        id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    })
}

/// One cover fetch per track, not per frame: the map is the cache, and a track
/// that has no art is simply asked for again when it comes back around.
fn ensure_art(deck: &gpui::Entity<Deck>, cx: &mut HeadlessAppContext) -> bool {
    let wanted = deck.update(cx, |deck, _| {
        deck.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.current.as_ref())
            .filter(|track| track.side == Side::Local && !track.id.is_empty())
            .filter(|track| !deck.art.contains_key(&track.id))
            .map(|track| track.id.clone())
    });
    let Some(cid) = wanted else {
        return false;
    };
    let Some(image) = fetch_art(&cid) else {
        return false;
    };
    deck.update(cx, |deck, cx| {
        deck.set_art(cid, Arc::new(image));
        cx.notify();
    });
    true
}

/// One bounded request to the catalog. Everything the Spore knows about the
/// playhead comes through here now: the host this package runs on carries
/// `soil:http`, not `soil:playback`, and `deadwax serve` already exposes MPD.
fn http_json(
    method: http::Method,
    path: &str,
    body: Option<Vec<u8>>,
) -> Result<serde_json::Value, String> {
    let response = http::fetch(&http::Request {
        url: format!("{CATALOG}{path}"),
        method,
        headers: vec![],
        body,
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: TIMEOUT_MS,
    })
    .map_err(|error| format!("{path}: {error:?}"))?;
    if !(200..300).contains(&response.status) {
        return Err(format!("{path}: HTTP {}", response.status));
    }
    if response.body.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_slice(&response.body).map_err(|error| format!("{path} is not JSON: {error}"))
}

fn post(path: &str) -> Result<(), String> {
    http_json(http::Method::Post, path, None).map(|_| ())
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// MPD's status and queue, as the catalog reports them, folded into one
/// snapshot. A queue entry is named by the URI MPD uses — the catalog resolves
/// it to bytes, this view never does.
fn fetch_snapshot() -> Result<Option<Snapshot>, String> {
    let status = http_json(http::Method::Get, "/mpd/status", None)?;
    let uri = text(&status, "uri");
    if uri.is_empty() {
        return Ok(None);
    }
    let queue = http_json(http::Method::Get, "/mpd/queue", None)?;
    let items = queue
        .get("items")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut tracks: Vec<Track> = items
        .iter()
        .map(|item| Track {
            side: Side::Local,
            id: text(item, "uri"),
            title: text(item, "title"),
            artist: text(item, "artist"),
            album: item
                .get("album")
                .and_then(serde_json::Value::as_str)
                .filter(|album| !album.is_empty())
                .map(str::to_string),
            duration_ms: None,
            art: None,
            playable: true,
        })
        .collect();
    let duration_ms = status
        .get("duration")
        .and_then(serde_json::Value::as_f64)
        .map(|seconds| (seconds * 1000.0) as u64);
    let current_index = items.iter().position(|item| {
        item.get("current")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    });
    if let (Some(index), Some(duration)) = (current_index, duration_ms)
        && let Some(track) = tracks.get_mut(index)
    {
        track.duration_ms = Some(duration);
    }
    let current = current_index
        .and_then(|index| tracks.get(index).cloned())
        .or_else(|| {
            Some(Track {
                side: Side::Local,
                id: uri.clone(),
                title: text(&status, "title"),
                artist: text(&status, "artist"),
                album: status
                    .get("album")
                    .and_then(serde_json::Value::as_str)
                    .filter(|album| !album.is_empty())
                    .map(str::to_string),
                duration_ms,
                art: None,
                playable: true,
            })
        });
    Ok(Some(Snapshot {
        revision: 0,
        side: Side::Local,
        playing: status
            .get("playing")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        position_ms: status
            .get("elapsed")
            .and_then(serde_json::Value::as_f64)
            .map(|elapsed| (elapsed * 1000.0) as u64)
            .unwrap_or(0),
        volume: status
            .get("volume")
            .and_then(serde_json::Value::as_u64)
            .map(|level| level as f32 / 100.0),
        repeat: view::RepeatMode::Off,
        shuffle: false,
        current,
        index: current_index.map(|index| index as u32),
        queue: tracks,
    }))
}

fn present_frame(
    context: &WgpuContext,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> anyhow::Result<()> {
    let texture = context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deadwax frame"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    context.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let texture = wgpu_wasi_webgpu::host_texture(texture);
    surface::present(&texture).map_err(|error| anyhow::anyhow!("present: {error}"))?;
    Ok(())
}

/// Where the list pane starts, shared with the store's hit test.
const LIST_TOP: f32 = view::HEADER_H + view::QUERY_H;

fn row_at(count: usize, width: f32, x: f32, y: f32) -> Option<usize> {
    if x >= width - view::SIDE_W || y < LIST_TOP {
        return None;
    }
    let index = ((y - LIST_TOP) / view::ROW_H) as usize;
    (index < count).then_some(index)
}

/// Re-read the playhead after a command. The catalog shells to MPD, so the
/// answer is immediate; there is no bus to wait on and nothing to reconcile.
fn refresh(deck: &gpui::Entity<Deck>, cx: &mut HeadlessAppContext) {
    let snapshot = fetch_snapshot();
    deck.update(cx, |deck, cx| {
        match snapshot {
            Ok(Some(snapshot)) => deck.apply_snapshot(snapshot),
            Ok(None) => deck.snapshot = None,
            Err(error) => deck.fail(error),
        }
        cx.notify();
    });
}

fn run_request(
    deck: &gpui::Entity<Deck>,
    request: Request,
    context: &mut HeadlessAppContext,
) -> anyhow::Result<()> {
    let transport = !matches!(
        request,
        Request::Search(..)
            | Request::Refresh
            | Request::RefreshPlaylists
            | Request::RefreshWishlist
            | Request::CycleWish(..)
            | Request::RemoveWish(..)
    );
    match request {
        Request::Toggle => report(deck, context, post("/mpd/toggle")),
        Request::Next => report(deck, context, post("/mpd/next")),
        Request::Previous => report(deck, context, post("/mpd/prev")),
        Request::Play(track) => {
            // A catalog row: append it to the shared queue, which is where the
            // native controller would put it too.
            if !track.playable {
                notice(deck, context, "that one is not digitized yet");
            } else {
                let path = format!("/mpd/add?uri={}", urlencode(&track.id));
                report(deck, context, post(&path));
                notice(deck, context, &format!("queued {}", track.title));
            }
        }
        Request::PlayFrom(index) => {
            report(
                deck,
                context,
                post(&format!("/mpd/play?position={}", index + 1)),
            );
        }
        Request::Seek(seconds) => {
            report(
                deck,
                context,
                post(&format!("/mpd/seek?seconds={seconds}")),
            );
        }
        Request::Volume(level) => {
            let percent = (level.clamp(0.0, 1.0) * 100.0).round() as u8;
            report(
                deck,
                context,
                post(&format!("/mpd/volume?level={percent}")),
            );
        }
        Request::PlayPlaylist(owner, name) => match play_playlist(&owner, &name) {
            Ok(()) => notice(deck, context, &format!("queued {name}")),
            Err(error) => failure(deck, context, &error),
        },
        Request::RefreshWishlist => match fetch_wishlist() {
            Ok(wishlist) => deck.update(context, |deck, cx| {
                deck.wishlist = wishlist;
                cx.notify();
            }),
            Err(error) => failure(deck, context, &error),
        },
        Request::CycleWish(id, status) => {
            let path = format!("/wishlist/status?id={id}&status={status}");
            report(deck, context, post(&path));
            run_request(deck, Request::RefreshWishlist, context)?;
        }
        Request::RemoveWish(id) => {
            report(deck, context, post(&format!("/wishlist/remove?id={id}")));
            run_request(deck, Request::RefreshWishlist, context)?;
        }
        Request::RefreshPlaylists => match fetch_playlists() {
            Ok(playlists) => deck.update(context, |deck, cx| {
                deck.playlists = playlists;
                cx.notify();
            }),
            Err(error) => deck.update(context, |deck, cx| {
                deck.fail(error);
                cx.notify();
            }),
        },
        Request::SelectSource(side) => {
            // One source now: the catalog. Spotify would need an account, which
            // lives behind `deadwax serve` and the native controller.
            deck.update(context, |deck, cx| {
                deck.side = side;
                cx.notify();
            });
        }
        Request::Enqueue(track) => {
            // "Queue this one": the same route as play, without starting it.
            let path = format!("/mpd/add?uri={}", urlencode(&track.id));
            match post(&path) {
                Ok(()) => notice(deck, context, &format!("queued {}", track.title)),
                Err(error) => failure(deck, context, &error),
            }
        }
        Request::Focus(..) | Request::RefreshSessions => {}
        Request::Search(side, query) => {
            let result = match side {
                Side::Local => fetch_catalog(&query),
                Side::Spotify => Err("spotify search needs the native controller".to_string()),
            };
            deck.update(context, |deck, cx| {
                match result {
                    Ok(tracks) => deck.set_results(side, tracks),
                    Err(error) => deck.fail(error),
                }
                cx.notify();
            });
        }
        Request::Refresh => {
            let (side, query) = deck.update(context, |deck, _| (deck.side, deck.query.clone()));
            run_request(deck, Request::Search(side, query), context)?;
        }
    }
    if transport {
        refresh(deck, context);
    }
    Ok(())
}

fn report(deck: &gpui::Entity<Deck>, cx: &mut HeadlessAppContext, result: Result<(), String>) {
    if let Err(error) = result {
        failure(deck, cx, &error);
    }
}

fn failure(deck: &gpui::Entity<Deck>, cx: &mut HeadlessAppContext, error: &str) {
    let error = error.to_string();
    deck.update(cx, |deck, cx| {
        deck.fail(error);
        cx.notify();
    });
}

fn notice(deck: &gpui::Entity<Deck>, cx: &mut HeadlessAppContext, text: &str) {
    let text = text.to_string();
    deck.update(cx, |deck, cx| {
        deck.notice = Some(text.into());
        cx.notify();
    });
}

fn render_frame() -> anyhow::Result<()> {
    let (width, height) = surface::size();
    let instance = wgpu_wasi_webgpu::instance();
    let adapter = wit_bindgen::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = wit_bindgen::block_on(adapter.request_device(&Default::default()))?;
    let (wgpu_context, format) = WgpuContext::from_offscreen(instance, adapter, device, queue)?;

    let text = Arc::new(CosmicTextSystem::new_without_system_fonts("Lilex"));
    text.add_fonts(vec![Cow::Borrowed(LILEX), Cow::Borrowed(LILEX_BOLD)])?;

    let factory_context = wgpu_context.clone();
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(()), move || {
        Ok(Some(Box::new(WgpuHeadlessRenderer::with_context(
            factory_context.clone(),
            format,
        )?)))
    });

    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
    });

    let deck = cx.new(|_| {
        let mut deck = Deck::new();
        // No `sources` call: the catalog is the only source this build can
        // reach, and the view falls back to local + spotify when the list is
        // empty (spotify answers with "needs the native controller").
        deck.requests
            .push(Request::Search(Side::Local, String::new()));
        deck.requests.push(Request::RefreshPlaylists);
        deck.requests.push(Request::RefreshWishlist);
        deck
    });

    // The volume thumb is a GPUI Kit widget, so it needs an entity and real
    // pointer events. `render_frame` forwards the presenter's pointer stream as
    // `PlatformInput`, which is what makes any kit control work in a Spore.
    let volume = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.01));
    deck.update(&mut cx, |deck, _| deck.volume_slider = Some(volume.clone()));
    let _volume_events = deck.update(&mut cx, |_, cx| {
        cx.subscribe(
            &volume,
            |deck, _slider, event: &SliderEvent, _cx| match event {
                SliderEvent::Change(_) => deck.dragging_volume = true,
                SliderEvent::Release(value) => {
                    deck.dragging_volume = false;
                    deck.set_volume(value.start());
                }
            },
        )
    });

    let seek = cx.new(|_| SliderState::new().min(0.).max(1.).step(0.001));
    deck.update(&mut cx, |deck, _| deck.seek_slider = Some(seek.clone()));
    let _seek_events = deck.update(&mut cx, |_, cx| {
        cx.subscribe(&seek, |deck, _slider, event: &SliderEvent, _cx| match event {
            SliderEvent::Change(_) => deck.dragging_seek = true,
            SliderEvent::Release(value) => {
                deck.dragging_seek = false;
                deck.seek_to(value.start());
            }
        })
    });

    let scale = surface::scale() as f32;
    let window = cx.open_window_scaled(size(px(width as f32), px(height as f32)), scale, {
        let deck = deck.clone();
        move |window, cx| cx.new(|cx| gpui_component::Root::new(deck, window, cx))
    })?;
    let any: AnyWindowHandle = window.into();

    let mut window_size = (width, height);
    let mut dirty = true;
    let mut pointer_down = false;
    let mut last_poll = std::time::Instant::now() - std::time::Duration::from_secs(1);
    loop {
        let size = surface::size();
        if size != window_size {
            window_size = size;
            window.update(&mut cx, |_, window, _| {
                window.resize(gpui::size(px(size.0 as f32), px(size.1 as f32)));
            })?;
            dirty = true;
        }

        // Poll the playhead once a second: MPD is the clock, and one HTTP pair
        // per second is cheaper than a frame-by-frame question.
        if last_poll.elapsed() >= std::time::Duration::from_millis(1000) {
            refresh(&deck, &mut cx);
            last_poll = std::time::Instant::now();
            dirty = true;
        }

        let requests = deck.update(&mut cx, |deck, _| std::mem::take(&mut deck.requests));
        if !requests.is_empty() {
            for request in requests {
                run_request(&deck, request, &mut cx)?;
            }
            dirty = true;
        }

        // A new current track means a new cover; fetch it once, before drawing.
        if ensure_art(&deck, &mut cx) {
            dirty = true;
        }

        if dirty {
            cx.update_window(any, |_, window, app| {
                let _ = window.draw(app);
            })?;
            let image = cx.capture_screenshot(any)?;
            present_frame(&wgpu_context, image.width(), image.height(), image.as_raw())?;
            dirty = false;
        }

        if !input::interactive() {
            break;
        }
        while let Some(event) = input::poll(0) {
            match event {
                input::Event::KeyInput(key) => {
                    let name = key.key.to_ascii_lowercase();
                    let editing = deck.update(&mut cx, |deck, _| deck.query_open);
                    if editing {
                        match name.as_str() {
                            "escape" => {
                                deck.update(&mut cx, |deck, cx| {
                                    deck.query_open = false;
                                    cx.notify();
                                });
                            }
                            "enter" => {
                                let (side, query) = deck.update(&mut cx, |deck, cx| {
                                    deck.query_open = false;
                                    cx.notify();
                                    (deck.side, deck.query.clone())
                                });
                                run_request(&deck, Request::Search(side, query), &mut cx)?;
                            }
                            "backspace" => {
                                deck.update(&mut cx, |deck, cx| {
                                    deck.query.pop();
                                    cx.notify();
                                });
                            }
                            "arrow-down" => {
                                deck.update(&mut cx, |deck, cx| {
                                    deck.move_selection(1);
                                    cx.notify();
                                });
                            }
                            "arrow-up" => {
                                deck.update(&mut cx, |deck, cx| {
                                    deck.move_selection(-1);
                                    cx.notify();
                                });
                            }
                            _ => {
                                if let Some(text) = key.text.clone()
                                    && !key.control
                                    && !key.meta
                                    && !text.chars().any(char::is_control)
                                {
                                    deck.update(&mut cx, |deck, cx| {
                                        deck.query.push_str(&text);
                                        cx.notify();
                                    });
                                }
                            }
                        }
                        dirty = true;
                        continue;
                    }
                    match name.as_str() {
                        "/" => {
                            deck.update(&mut cx, |deck, cx| {
                                deck.query_open = true;
                                deck.query.clear();
                                cx.notify();
                            });
                            dirty = true;
                        }
                        "enter" => {
                            let track = deck.update(&mut cx, |deck, _| deck.selected_track());
                            if let Some(track) = track {
                                run_request(&deck, Request::Play(track), &mut cx)?;
                            }
                            dirty = true;
                        }
                        "a" => {
                            let track = deck.update(&mut cx, |deck, _| deck.selected_track());
                            if let Some(track) = track {
                                run_request(&deck, Request::Enqueue(track), &mut cx)?;
                            }
                            dirty = true;
                        }
                        " " | "space" => {
                            run_request(&deck, Request::Toggle, &mut cx)?;
                            dirty = true;
                        }
                        "n" | "arrow-down" if !key.control => {
                            if key.key.eq_ignore_ascii_case("arrow-down") {
                                deck.update(&mut cx, |deck, cx| {
                                    deck.move_selection(1);
                                    cx.notify();
                                });
                            } else {
                                run_request(&deck, Request::Next, &mut cx)?;
                            }
                            dirty = true;
                        }
                        "p" | "arrow-up" if !key.control => {
                            if key.key.eq_ignore_ascii_case("arrow-up") {
                                deck.update(&mut cx, |deck, cx| {
                                    deck.move_selection(-1);
                                    cx.notify();
                                });
                            } else {
                                run_request(&deck, Request::Previous, &mut cx)?;
                            }
                            dirty = true;
                        }
                        "j" => {
                            deck.update(&mut cx, |deck, cx| {
                                deck.move_selection(1);
                                cx.notify();
                            });
                            dirty = true;
                        }
                        "k" => {
                            deck.update(&mut cx, |deck, cx| {
                                deck.move_selection(-1);
                                cx.notify();
                            });
                            dirty = true;
                        }
                        "s" => {
                            let (current, sources) =
                                deck.update(&mut cx, |deck, _| (deck.side, deck.sources.clone()));
                            let target = match current {
                                Side::Local => Side::Spotify,
                                Side::Spotify => Side::Local,
                            };
                            let capability =
                                sources.iter().find(|source| source.side == target).cloned();
                            match capability {
                                Some(capability) => {
                                    deck.update(&mut cx, |deck, cx| {
                                        deck.side = target;
                                        deck.selected = 0;
                                        deck.results.clear();
                                        deck.notice = Some(
                                            format!(
                                                "{}{}",
                                                capability.label,
                                                if capability.can_search {
                                                    ""
                                                } else {
                                                    " · search is the owner's"
                                                }
                                            )
                                            .into(),
                                        );
                                        cx.notify();
                                    });
                                    run_request(&deck, Request::SelectSource(target), &mut cx)?;
                                    // The owned catalogue is searched by this Spore;
                                    // only a remote source needs the owner's search.
                                    if target == Side::Local || capability.can_search {
                                        let query =
                                            deck.update(&mut cx, |deck, _| deck.query.clone());
                                        run_request(
                                            &deck,
                                            Request::Search(target, query),
                                            &mut cx,
                                        )?;
                                    }
                                }
                                None => {
                                    deck.update(&mut cx, |deck, cx| {
                                        deck.notice = Some(
                                            format!("the owner has no {} source", target.label())
                                                .into(),
                                        );
                                        cx.notify();
                                    });
                                }
                            }
                            dirty = true;
                        }
                        "r" => {
                            run_request(&deck, Request::Refresh, &mut cx)?;
                            dirty = true;
                        }
                        "q" => return Ok(()),
                        _ => {}
                    }
                }
                // The presenter's pointer stream becomes GPUI input, so kit
                // controls (the volume thumb today, anything else later) get
                // real press/drag/release instead of geometry guesses here.
                input::Event::Moved(motion) => {
                    let event = PlatformInput::MouseMove(MouseMoveEvent {
                        position: point(px(motion.x), px(motion.y)),
                        pressed_button: pointer_down.then_some(MouseButton::Left),
                        modifiers: Default::default(),
                    });
                    let _ = window.update(&mut cx, |_, window, app| {
                        window.dispatch_event(event, app);
                    });
                    dirty = true;
                }
                input::Event::Button(button) => {
                    let left = button.button == input::PointerButton::Left;
                    if left {
                        pointer_down = button.pressed;
                    }
                    let position = point(px(button.x), px(button.y));
                    let event = if button.pressed {
                        PlatformInput::MouseDown(MouseDownEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        })
                    } else {
                        PlatformInput::MouseUp(MouseUpEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers: Default::default(),
                            click_count: 1,
                        })
                    };
                    let _ = window.update(&mut cx, |_, window, app| {
                        window.dispatch_event(event, app);
                    });
                    // The results list is still plain divs with a geometric hit
                    // test; selection stays here until it becomes a kit list.
                    if button.pressed && left {
                        let width = surface::size().0 as f32;
                        let count = deck.update(&mut cx, |deck, _| deck.results.len());
                        if let Some(index) = row_at(count, width, button.x, button.y) {
                            deck.update(&mut cx, |deck, cx| {
                                deck.selected = index;
                                cx.notify();
                            });
                        }
                    }
                    dirty = true;
                }
                _ => {}
            }
        }

        // Sleep until the next event or a tick, so an idle controller costs
        // one wake-up every 8 ms instead of a busy loop. The host waits the
        // timeout even with no owner attached, and an event that arrives during
        // the wait is a real update: dropping it would lose the first snapshot
        // after a quiet poll.
        let _ = input::poll(8);
    }
    Ok(())
}

struct Component;

impl Guest for Component {
    async fn run() -> u32 {
        match render_frame() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("deadwax: {error:#}");
                1
            }
        }
    }
}

export!(Component);
