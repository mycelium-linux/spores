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
use soil::playback::types as playback;
use soil::playback::{desk, events, player};
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

fn error_text(error: &playback::Error) -> &'static str {
    match error {
        playback::Error::Denied => "denied",
        playback::Error::Unavailable => "no playback owner",
        playback::Error::Invalid => "invalid request",
        playback::Error::Stale => "the queue moved on",
        playback::Error::Unsupported => "the owner cannot do that",
        playback::Error::Failed => "playback failed",
        playback::Error::Blocked => "the studio has the speakers",
    }
}

fn side_of(source: playback::Source) -> Side {
    match source {
        playback::Source::Local => Side::Local,
        playback::Source::Spotify => Side::Spotify,
    }
}

fn source_of(side: Side) -> playback::Source {
    match side {
        Side::Local => playback::Source::Local,
        Side::Spotify => playback::Source::Spotify,
    }
}

fn track_of(track: &playback::Track) -> Track {
    Track {
        side: side_of(track.source),
        id: track.id.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration_ms: track.duration_ms,
        art: track.art.clone(),
        playable: true,
    }
}

fn snapshot_of(state: &playback::State) -> Snapshot {
    Snapshot {
        revision: state.revision,
        side: side_of(state.source),
        playing: state.playing,
        position_ms: state.position_ms,
        volume: state.volume,
        repeat: match state.repeat {
            playback::Repeat::Off => view::RepeatMode::Off,
            playback::Repeat::Track => view::RepeatMode::Track,
            playback::Repeat::Queue => view::RepeatMode::Queue,
        },
        shuffle: state.shuffle,
        current: state.current.as_ref().map(track_of),
        index: state.index,
        queue: state
            .queue
            .iter()
            .map(|entry| track_of(&entry.track))
            .collect(),
    }
}

fn playback_track(track: &Track) -> playback::Track {
    playback::Track {
        source: source_of(track.side),
        id: track.id.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration_ms: track.duration_ms,
        art: track.art.clone(),
    }
}

fn queue_entry(track: &Track) -> playback::QueueEntry {
    playback::QueueEntry {
        track: playback_track(track),
        requested_by: None,
    }
}

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

fn sessions_now() -> Vec<view::SessionChip> {
    desk::sessions()
        .into_iter()
        .map(|session| view::SessionChip {
            owner: session.target.owner.clone(),
            session: session.target.session.clone(),
            label: session.label,
            kind: session.kind,
            playing: session.playing,
            focused: session.focused,
            capability: view::SessionCapability {
                can_play: session.capability.can_play,
                can_pause: session.capability.can_pause,
                can_next: session.capability.can_next,
                can_previous: session.capability.can_previous,
                can_seek: session.capability.can_seek,
                has_volume: session.capability.has_volume,
                can_search: session.capability.can_search,
            },
        })
        .collect()
}

fn search_spotify(query: &str) -> Result<Vec<Track>, String> {
    let tracks = player::search(playback::Source::Spotify, query, PAGE)
        .map_err(|error| error_text(&error).to_string())?;
    Ok(tracks.iter().map(track_of).collect())
}

fn send(command: playback::Command) -> Result<u64, String> {
    player::send(&command).map_err(|error| error_text(&error).to_string())
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

/// Re-read the owner after a command. A host that publishes on Unibus may take
/// a moment to send the matching snapshot; a host that executes inline answers
/// immediately, and neither case needs a second code path in the guest.
fn refresh(deck: &gpui::Entity<Deck>, cx: &mut HeadlessAppContext) {
    if let Ok(state) = player::snapshot() {
        deck.update(cx, |deck, cx| {
            deck.apply_snapshot(snapshot_of(&state));
            cx.notify();
        });
    }
    let sessions = sessions_now();
    deck.update(cx, |deck, cx| {
        deck.set_sessions(sessions);
        cx.notify();
    });
}

fn apply_event(deck: &gpui::Entity<Deck>, cx: &mut HeadlessAppContext, event: playback::Event) {
    match event {
        playback::Event::Snapshot(state) => {
            let snapshot = snapshot_of(&state);
            deck.update(cx, |deck, cx| {
                deck.apply_snapshot(snapshot);
                cx.notify();
            });
        }
        playback::Event::Progress(position) => {
            deck.update(cx, |deck, cx| {
                deck.apply_progress(position.revision, position.position_ms, position.playing);
                cx.notify();
            });
        }
        playback::Event::Volume(level) => {
            deck.update(cx, |deck, cx| {
                deck.apply_volume(level);
                cx.notify();
            });
        }
        playback::Event::Failed(reason) => {
            // A studio lockout is not a broken player: the queue is fine, it is
            // just not audible. Say so without reddening the transport.
            if matches!(reason, playback::Error::Blocked) {
                deck.update(cx, |deck, cx| {
                    deck.notice = Some("the studio has the speakers".into());
                    cx.notify();
                });
            } else {
                deck.update(cx, |deck, cx| {
                    deck.fail(error_text(&reason));
                    cx.notify();
                });
            }
        }
        playback::Event::Focus(_) => {
            let sessions = sessions_now();
            deck.update(cx, |deck, cx| {
                deck.set_sessions(sessions);
                cx.notify();
            });
            refresh(deck, cx);
        }
    }
}

fn run_request(
    deck: &gpui::Entity<Deck>,
    request: Request,
    context: &mut HeadlessAppContext,
) -> anyhow::Result<()> {
    let transport = !matches!(
        request,
        Request::Search(..) | Request::Refresh | Request::RefreshPlaylists
    );
    match request {
        Request::Toggle => {
            if let Err(error) = send(playback::Command::Toggle) {
                deck.update(context, |deck, cx| {
                    deck.fail(error);
                    cx.notify();
                });
            }
        }
        Request::Next => {
            let _ = send(playback::Command::Next);
        }
        Request::Previous => {
            let _ = send(playback::Command::Previous);
        }
        Request::Play(track) => {
            if !track.playable {
                deck.update(context, |deck, cx| {
                    deck.notice = Some("that one is not digitized yet".into());
                    cx.notify();
                });
            } else {
                let command = playback::Command::SetQueue(playback::QueueRequest {
                    entries: vec![queue_entry(&track)],
                    start: 0,
                });
                if let Err(error) = send(command) {
                    deck.update(context, |deck, cx| {
                        deck.fail(error);
                        cx.notify();
                    });
                }
            }
        }
        Request::Enqueue(track) => {
            if !track.playable {
                deck.update(context, |deck, cx| {
                    deck.notice = Some("that one is not digitized yet".into());
                    cx.notify();
                });
            } else {
                let command = playback::Command::Enqueue(playback::EnqueueRequest {
                    entry: queue_entry(&track),
                    next: false,
                });
                match send(command) {
                    Ok(_) => deck.update(context, |deck, cx| {
                        deck.notice = Some(format!("queued {}", track.title).into());
                        cx.notify();
                    }),
                    Err(error) => deck.update(context, |deck, cx| {
                        deck.fail(error);
                        cx.notify();
                    }),
                }
            }
        }
        Request::SelectSource(side) => {
            let _ = send(playback::Command::SelectSource(source_of(side)));
        }
        Request::Focus(owner, session) => {
            let target = playback::SessionRef { owner, session };
            if let Err(error) = desk::focus(&target) {
                deck.update(context, |deck, cx| {
                    deck.fail(error_text(&error));
                    cx.notify();
                });
            }
            let sessions = sessions_now();
            deck.update(context, |deck, cx| {
                deck.set_sessions(sessions);
                cx.notify();
            });
        }
        Request::Volume(level) => {
            if let Err(error) = send(playback::Command::Volume(level)) {
                deck.update(context, |deck, cx| {
                    deck.fail(error);
                    cx.notify();
                });
            }
        }
        Request::PlayFrom(index) => {
            let queue = deck.update(context, |deck, _| {
                deck.snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.queue.clone())
                    .unwrap_or_default()
            });
            let entries: Vec<playback::QueueEntry> =
                queue.iter().skip(index).map(queue_entry).collect();
            if entries.is_empty() {
                deck.update(context, |deck, cx| {
                    deck.notice = Some("nothing queued from there".into());
                    cx.notify();
                });
            } else if let Err(error) =
                send(playback::Command::SetQueue(playback::QueueRequest {
                    entries,
                    start: 0,
                }))
            {
                deck.update(context, |deck, cx| {
                    deck.fail(error);
                    cx.notify();
                });
            }
        }
        Request::PlayPlaylist(owner, name) => {
            match play_playlist(&owner, &name) {
                Ok(()) => deck.update(context, |deck, cx| {
                    deck.notice = Some(format!("queued {name}").into());
                    cx.notify();
                }),
                Err(error) => deck.update(context, |deck, cx| {
                    deck.fail(error);
                    cx.notify();
                }),
            }
        }
        Request::RefreshPlaylists => {
            let playlists = fetch_playlists();
            deck.update(context, |deck, cx| {
                match playlists {
                    Ok(playlists) => deck.playlists = playlists,
                    Err(error) => deck.fail(error),
                }
                cx.notify();
            });
        }
        Request::RefreshSessions => {
            let sessions = sessions_now();
            deck.update(context, |deck, cx| {
                deck.set_sessions(sessions);
                cx.notify();
            });
        }
        Request::Search(side, query) => {
            deck.update(context, |deck, cx| {
                deck.loading = true;
                deck.error = None;
                cx.notify();
            });
            let result = match side {
                Side::Local => fetch_catalog(&query),
                Side::Spotify => search_spotify(&query),
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
        deck.sources = player::sources()
            .iter()
            .map(|source| view::Capability {
                side: side_of(source.source),
                label: source.label.clone(),
                can_search: source.can_search,
            })
            .collect();
        match player::snapshot() {
            Ok(state) => deck.apply_snapshot(snapshot_of(&state)),
            Err(error) => deck.fail(error_text(&error)),
        }
        deck.set_sessions(sessions_now());
        deck.requests
            .push(Request::Search(Side::Local, String::new()));
        deck.requests.push(Request::RefreshPlaylists);
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

    let scale = surface::scale() as f32;
    let window = cx.open_window_scaled(size(px(width as f32), px(height as f32)), scale, {
        let deck = deck.clone();
        move |window, cx| cx.new(|cx| gpui_component::Root::new(deck, window, cx))
    })?;
    let any: AnyWindowHandle = window.into();

    let mut window_size = (width, height);
    let mut dirty = true;
    let mut pointer_down = false;
    loop {
        let size = surface::size();
        if size != window_size {
            window_size = size;
            window.update(&mut cx, |_, window, _| {
                window.resize(gpui::size(px(size.0 as f32), px(size.1 as f32)));
            })?;
            dirty = true;
        }

        // Playback events first: a track change should be painted before a
        // keystroke that arrived in the same frame.
        if events::attached() {
            while let Some(event) = events::poll(0) {
                apply_event(&deck, &mut cx, event);
                dirty = true;
            }
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
        if let Some(event) = events::poll(8) {
            apply_event(&deck, &mut cx, event);
            dirty = true;
        }
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
