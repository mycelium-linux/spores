//! The controller's view: pure GPUI, no transport.
//!
//! `lib.rs` converts `soil:playback` records into the snapshot below and feeds
//! it here, the way `terminal-gpui` converts a host grid. That keeps the whole
//! layout testable from a fixture and keeps every WIT type out of the paint
//! code.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, Context, Entity, Image, IntoElement, ObjectFit, Render, SharedString, StyledImage as _,
    Window, div, img, prelude::*, px, rgb,
};
use gpui_component::slider::{Slider, SliderState};
use gpui_component::tag::Tag;

const BG: u32 = 0x0c0a08;
const PANEL: u32 = 0x14110d;
const PANEL_HI: u32 = 0x1d1913;
const ROW_HOVER: u32 = 0x211b13;
const LINE: u32 = 0x2a241c;
const TEXT: u32 = 0xe9dfcc;
const MUTED: u32 = 0x8a7d69;
const FAINT: u32 = 0x5c5346;
const ACCENT: u32 = 0xd9a441;
const OK: u32 = 0x7fb069;
const BAD: u32 = 0xd1604b;

pub const HEADER_H: f32 = 54.0;
pub const FOOTER_H: f32 = 26.0;
pub const SIDE_W: f32 = 372.0;
/// The search bar above the list. Shared with the pointer hit test.
pub const QUERY_H: f32 = 38.0;
pub const ROW_H: f32 = 46.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Local,
    Spotify,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "library",
            Self::Spotify => "spotify",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RepeatMode {
    Off,
    Track,
    Queue,
}

/// One item in the list, whether it came from the catalog or from the owner's
/// Spotify search. `playable` is false for a discogs track that is not
/// digitized yet: it is worth showing, but there is nothing to hand the owner.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub side: Side,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub art: Option<String>,
    pub playable: bool,
}

/// An album a playlist wants that the library does not have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingAlbum {
    pub artist: String,
    pub album: String,
    pub missing: usize,
}

/// One imported playlist as `deadwax serve` reports it with coverage: the list,
/// its owner, and how much of it the library already owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playlist {
    pub owner: String,
    pub name: String,
    pub tracks: usize,
    pub owned: usize,
    /// The albums that would shorten the list most, already ranked by the
    /// server. Trimmed to a few: this panel is not a shopping list.
    pub missing: Vec<MissingAlbum>,
}

impl Playlist {
    pub fn missing_tracks(&self) -> usize {
        self.tracks.saturating_sub(self.owned)
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub revision: u64,
    pub side: Side,
    pub playing: bool,
    pub position_ms: u64,
    pub volume: Option<f32>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub current: Option<Track>,
    pub index: Option<u32>,
    pub queue: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    pub side: Side,
    pub label: String,
    pub can_search: bool,
}

/// What one active session can honour. A global verb that is not supported
/// must be greyed out, not reinterpreted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionCapability {
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    pub has_volume: bool,
    pub can_search: bool,
}

/// One active concept on the desk. Several may play at once; exactly one is
/// focused for global transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionChip {
    pub owner: String,
    pub session: String,
    pub label: String,
    pub kind: String,
    pub playing: bool,
    pub focused: bool,
    pub capability: SessionCapability,
}

/// What the view asks the host to do after a frame. The view never calls
/// `soil:playback` itself; `lib.rs` drains this list and issues the command.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Toggle,
    Next,
    Previous,
    Play(Track),
    Enqueue(Track),
    SelectSource(Side),
    Search(Side, String),
    /// Ask the desk to send global transport to this session.
    Focus(String, String),
    /// Normalized `0..=1`, the unit the playback contract speaks.
    Volume(f32),
    /// Play the queue from this index, keeping the rest of it.
    PlayFrom(usize),
    /// Replace the shared queue with an imported playlist and start it.
    PlayPlaylist(String, String),
    RefreshPlaylists,
    RefreshSessions,
    Refresh,
}

pub struct Deck {
    pub snapshot: Option<Snapshot>,
    pub sources: Vec<Capability>,
    /// Every active concept the desk reports, focused first.
    pub sessions: Vec<SessionChip>,
    pub focused: Option<(String, String)>,
    /// The side being browsed. Playback can be on the other one; the header
    /// says which is which.
    pub side: Side,
    pub results: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub query: String,
    pub query_open: bool,
    pub selected: usize,
    pub notice: Option<SharedString>,
    pub error: Option<SharedString>,
    pub fetched_at: Option<Instant>,
    pub loading: bool,
    pub requests: Vec<Request>,
    /// Covers fetched by content address, keyed by the CID the queue entry
    /// carries. A redraw never re-fetches; a missing key draws the vinyl disc.
    pub art: HashMap<String, Arc<Image>>,
    /// The volume thumb, when `lib.rs` attached one. It stays optional so the
    /// view can still be built from a fixture in a test, with no app context.
    pub volume_slider: Option<Entity<SliderState>>,
    /// True while the pointer holds the volume thumb: host updates must not
    /// fight the hand that is dragging.
    pub dragging_volume: bool,
}

impl Default for Deck {
    fn default() -> Self {
        Self::new()
    }
}

impl Deck {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            sources: Vec::new(),
            sessions: Vec::new(),
            focused: None,
            side: Side::Local,
            results: Vec::new(),
            playlists: Vec::new(),
            query: String::new(),
            query_open: false,
            selected: 0,
            notice: None,
            error: None,
            fetched_at: None,
            loading: false,
            requests: Vec::new(),
            art: HashMap::new(),
            volume_slider: None,
            dragging_volume: false,
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: Snapshot) {
        // Follow the owner's source on the first snapshot so a person who
        // started playback elsewhere sees what they are hearing.
        if self.snapshot.is_none() {
            self.side = snapshot.side;
        }
        self.snapshot = Some(snapshot);
        self.error = None;
    }

    pub fn apply_progress(&mut self, revision: u64, position_ms: u64, playing: bool) {
        if let Some(snapshot) = &mut self.snapshot
            && revision >= snapshot.revision
        {
            snapshot.revision = revision;
            snapshot.position_ms = position_ms;
            snapshot.playing = playing;
        }
    }

    /// Ask the host for a volume change. One place, so every caller (the thumb,
    /// a key, a future remote) sends the same command.
    pub fn set_volume(&mut self, level: f32) {
        let level = level.clamp(0.0, 1.0);
        self.apply_volume(level);
        self.requests.push(Request::Volume(level));
    }
    /// The thumbnail's value, kept in step with the owner unless the pointer is
    /// holding it.
    pub fn sync_volume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slider) = &self.volume_slider else {
            return;
        };
        if self.dragging_volume {
            return;
        }
        let Some(level) = self.snapshot.as_ref().and_then(|snapshot| snapshot.volume) else {
            return;
        };
        if (slider.read(cx).value().start() - level).abs() > 0.005 {
            slider.update(cx, |state, cx| state.set_value(level, window, cx));
        }
    }

    /// Fold the owner's volume into the snapshot without asking for a change.
    pub fn apply_volume(&mut self, level: f32) {
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.volume = Some(level);
        }
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionChip>) {
        self.focused = sessions
            .iter()
            .find(|session| session.focused)
            .map(|session| (session.owner.clone(), session.session.clone()));
        self.sessions = sessions;
    }

    pub fn set_results(&mut self, side: Side, results: Vec<Track>) {
        if self.side == side {
            self.results = results;
            self.selected = self.selected.min(self.results.len().saturating_sub(1));
            self.fetched_at = Some(Instant::now());
            self.loading = false;
        }
    }

    pub fn fail(&mut self, message: impl Into<SharedString>) {
        self.error = Some(message.into());
        self.loading = false;
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.results.is_empty() {
            return;
        }
        let last = self.results.len() - 1;
        let next = self.selected as i32 + delta;
        self.selected = next.clamp(0, last as i32) as usize;
    }

    pub fn selected_track(&self) -> Option<Track> {
        self.results.get(self.selected).cloned()
    }

    pub fn position_ms(&self) -> u64 {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.position_ms)
            .unwrap_or(0)
    }

    pub fn progress(&self) -> f32 {
        let Some(snapshot) = &self.snapshot else {
            return 0.0;
        };
        let Some(track) = &snapshot.current else {
            return 0.0;
        };
        match track.duration_ms {
            Some(duration) if duration > 0 => {
                (snapshot.position_ms as f32 / duration as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    fn request(&mut self, request: Request) {
        self.requests.push(request);
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let playing = self.snapshot.as_ref().map(|s| s.playing).unwrap_or(false);
        let status = match (&self.error, &self.snapshot) {
            (Some(_), _) => ("no owner", BAD),
            (None, Some(snapshot)) if snapshot.playing => ("playing", OK),
            (None, Some(_)) => ("paused", ACCENT),
            (None, None) => ("connecting", FAINT),
        };
        div()
            .h(px(HEADER_H))
            .flex_none()
            .px(px(16.0))
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(PANEL))
            .border_b_1()
            .border_color(rgb(LINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(TEXT))
                            .child("deadwax"),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(MUTED))
                            .child("the household playhead"),
                    ),
            )
            .child(self.sessions_strip(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded_full()
                            .bg(rgb(if playing { status.1 } else { FAINT })),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(status.1))
                            .child(status.0),
                    ),
            )
    }

    /// The active concepts. This is the desk, not a browse filter: deadwax,
    /// Spotify and a studio transport can all be here, and the focused one is
    /// what global transport addresses.
    fn sessions_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chips: Vec<AnyElement> = if self.sessions.is_empty() {
            vec![
                div()
                    .text_size(px(9.0))
                    .text_color(rgb(FAINT))
                    .child("no players")
                    .into_any_element(),
            ]
        } else {
            self.sessions
                .iter()
                .enumerate()
                .map(|(index, session)| self.session_chip(index, session, cx))
                .collect()
        };
        div().flex().items_center().gap(px(4.0)).children(chips)
    }

    fn session_chip(
        &self,
        index: usize,
        session: &SessionChip,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = session.focused;
        let owner = session.owner.clone();
        let id = session.session.clone();
        let playing = session.playing;
        div()
            .id(("session", index))
            .flex()
            .items_center()
            .gap(px(5.0))
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .bg(rgb(if focused { PANEL_HI } else { BG }))
            .border_1()
            .border_color(rgb(if focused { ACCENT } else { LINE }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _: &gpui::MouseDownEvent, _window, cx| {
                    this.request(Request::Focus(owner.clone(), id.clone()));
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(5.0))
                    .h(px(5.0))
                    .rounded(px(3.0))
                    .bg(rgb(if playing { OK } else { FAINT })),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(if focused { TEXT } else { MUTED }))
                    .child(SharedString::from(session.label.clone())),
            )
            .child(
                div()
                    .text_size(px(8.0))
                    .text_color(rgb(FAINT))
                    .child(SharedString::from(session.kind.clone())),
            )
            .into_any_element()
    }

    fn source_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sides: Vec<Side> = if self.sources.is_empty() {
            vec![Side::Local, Side::Spotify]
        } else {
            self.sources.iter().map(|source| source.side).collect()
        };
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .children(sides.into_iter().map(|side| {
                let active = side == self.side;
                div()
                    .id(("source", side as usize))
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .text_size(px(11.0))
                    .text_color(rgb(if active { TEXT } else { MUTED }))
                    .bg(rgb(if active { PANEL_HI } else { PANEL }))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _: &gpui::MouseDownEvent, _window, cx| {
                            this.side = side;
                            this.selected = 0;
                            this.results.clear();
                            this.request(Request::SelectSource(side));
                            // The library is searched here; a remote source is
                            // searched by the owner, and only when it says so.
                            if side == Side::Local
                                || this
                                    .sources
                                    .iter()
                                    .any(|source| source.side == side && source.can_search)
                            {
                                this.request(Request::Search(side, this.query.clone()));
                            }
                            cx.notify();
                        }),
                    )
                    .child(side.label())
            }))
    }

    fn library(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.results.len();
        div()
            .flex_1()
            .min_w(px(320.0))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(QUERY_H))
                    .flex_none()
                    .px(px(14.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(self.source_tabs(cx))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(rgb(if self.query_open { TEXT } else { FAINT }))
                            .child(if self.query_open {
                                SharedString::from(format!("/ {}", self.query))
                            } else {
                                SharedString::from("/  search the library")
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(MUTED))
                            .child(format!("{count} items")),
                    ),
            )
            .child(if self.results.is_empty() {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.0))
                    .text_color(rgb(FAINT))
                    .child(if self.loading {
                        "asking…"
                    } else if self.query.is_empty() {
                        "type to search"
                    } else {
                        "nothing matches"
                    })
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .overflow_hidden()
                    .children(
                        self.results
                            .iter()
                            .enumerate()
                            .map(|(index, track)| self.row(index, track, cx)),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn row(&self, index: usize, track: &Track, cx: &mut Context<Self>) -> AnyElement {
        let selected = index == self.selected;
        let playable = track.playable;
        div()
            .id(("row", index))
            .h(px(ROW_H))
            .flex_none()
            .px(px(14.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .bg(rgb(if selected { ROW_HOVER } else { BG }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _: &gpui::MouseDownEvent, _window, cx| {
                    this.selected = index;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(3.0))
                    .h(px(22.0))
                    .rounded(px(2.0))
                    .bg(rgb(if selected { ACCENT } else { BG })),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(if playable { TEXT } else { MUTED }))
                            .child(SharedString::from(track.title.clone())),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(FAINT))
                            .child(SharedString::from(track.artist.clone())),
                    ),
            )
            .child(if !playable {
                Tag::secondary().child("not digitized").into_any_element()
            } else if track.side == Side::Local {
                Tag::success().child("cid").into_any_element()
            } else {
                Tag::new().child("spotify").into_any_element()
            })
            .child(
                div()
                    .w(px(48.0))
                    .text_size(px(10.0))
                    .text_color(rgb(FAINT))
                    .child(track.duration_ms.map(clock).unwrap_or_default()),
            )
            .into_any_element()
    }

    fn now(&self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.snapshot.as_ref();
        let current = snapshot.and_then(|snapshot| snapshot.current.as_ref());
        let playing = snapshot.map(|snapshot| snapshot.playing).unwrap_or(false);
        let mut panel = div()
            .flex_none()
            .w(px(SIDE_W))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(rgb(LINE))
            .bg(rgb(PANEL))
            .child(self.disc(current));

        panel = panel.child(
            div()
                .px(px(18.0))
                .pb(px(12.0))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(px(15.0))
                        .text_color(rgb(TEXT))
                        .child(SharedString::from(
                            current
                                .map(|track| track.title.clone())
                                .unwrap_or_else(|| "nothing selected".into()),
                        )),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(MUTED))
                        .child(SharedString::from(
                            current
                                .map(|track| track.artist.clone())
                                .unwrap_or_default(),
                        )),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(FAINT))
                        .child(SharedString::from(
                            current
                                .and_then(|track| track.album.clone())
                                .unwrap_or_default(),
                        )),
                ),
        );

        // The playhead is two divs: a track and a filled portion. Nothing here
        // needs a kit slider, and a mis-drawn slider is worse than a bar.
        panel = panel.child(
            div()
                .px(px(18.0))
                .flex_none()
                .child(
                    div()
                        .h(px(3.0))
                        .w_full()
                        .rounded(px(2.0))
                        .bg(rgb(PANEL_HI))
                        .child(
                            div()
                                .h(px(3.0))
                                .rounded(px(2.0))
                                .bg(rgb(ACCENT))
                                .w(px((SIDE_W - 36.0) * self.progress())),
                        ),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .flex()
                        .justify_between()
                        .text_size(px(9.0))
                        .text_color(rgb(FAINT))
                        .child(clock(self.position_ms()))
                        .child(
                            current
                                .and_then(|track| track.duration_ms)
                                .map(clock)
                                .unwrap_or_else(|| "--:--".into()),
                        ),
                ),
        );

        panel = panel.child(
            div()
                .px(px(18.0))
                .py(px(10.0))
                .flex()
                .items_center()
                .gap(px(14.0))
                .child(self.transport_button("prev", "◀◀", cx, Request::Previous))
                .child(self.transport_button(
                    "toggle",
                    if playing { "❚❚" } else { "▶" },
                    cx,
                    Request::Toggle,
                ))
                .child(self.transport_button("next", "▶▶", cx, Request::Next))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(9.0))
                        .text_color(rgb(FAINT))
                        .child(match snapshot.map(|s| s.repeat) {
                            Some(RepeatMode::Track) => "repeat one".to_string(),
                            Some(RepeatMode::Queue) => "repeat queue".to_string(),
                            _ => String::new(),
                        }),
                )
                .child({
                    let mut row = div().flex().items_center().gap(px(6.0));
                    if let Some(slider) = &self.volume_slider {
                        row = row.child(div().w(px(96.0)).child(Slider::new(slider).horizontal()));
                    }
                    row.child(
                        div()
                            .text_size(px(9.0))
                            .text_color(rgb(FAINT))
                            .child(format!(
                                "vol {}",
                                snapshot
                                    .and_then(|s| s.volume)
                                    .map(|level| format!("{:.0}%", level * 100.0))
                                    .unwrap_or_else(|| "--".into())
                            )),
                    )
                }),
        );

        if !self.playlists.is_empty() {
            let mut section = div()
                .flex_none()
                .border_t_1()
                .border_color(rgb(LINE))
                .py(px(6.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(18.0))
                        .py(px(2.0))
                        .text_size(px(9.0))
                        .text_color(rgb(MUTED))
                        .child(format!("playlists · {}", self.playlists.len())),
                );
            for (index, playlist) in self.playlists.iter().take(4).enumerate() {
                let owner = playlist.owner.clone();
                let name = playlist.name.clone();
                section = section.child(
                    div()
                        .id(("playlist", index))
                        .px(px(18.0))
                        .py(px(4.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .hover(|style| style.bg(rgb(ROW_HOVER)))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(
                                move |this,
                                      _: &gpui::MouseDownEvent,
                                      _window,
                                      cx| {
                                    this.request(Request::PlayPlaylist(
                                        owner.clone(),
                                        name.clone(),
                                    ));
                                    cx.notify();
                                },
                            ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_size(px(10.0))
                                .text_color(rgb(TEXT))
                                .child(SharedString::from(playlist.name.clone())),
                        )
                        .child(Tag::secondary().child(SharedString::from(format!(
                            "{}/{}",
                            playlist.owned, playlist.tracks
                        )))),
                );
            }
            panel = panel.child(section);
        }

        panel = panel.child(
            div()
                .flex_1()
                .overflow_hidden()
                .border_t_1()
                .border_color(rgb(LINE))
                .child(
                    div()
                        .px(px(18.0))
                        .py(px(7.0))
                        .text_size(px(9.0))
                        .text_color(rgb(MUTED))
                        .child(format!(
                            "queue · {}",
                            snapshot.map(|s| s.queue.len()).unwrap_or(0)
                        )),
                )
                .children(
                    snapshot
                        .map(|s| s.queue.as_slice())
                        .unwrap_or_default()
                        .iter()
                        .enumerate()
                        .map(|(index, track)| {
                            let current_row = snapshot.and_then(|s| s.index) == Some(index as u32);
                            let playable = track.playable;
                            let target = track.clone();
                            div()
                                .id(("queue", index))
                                .px(px(18.0))
                                .py(px(4.0))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .bg(rgb(if current_row { PANEL_HI } else { PANEL }))
                                .hover(|style| style.bg(rgb(ROW_HOVER)))
                                .on_mouse_down(
                                    gpui::MouseButton::Left,
                                    cx.listener(
                                        move |this,
                                              _: &gpui::MouseDownEvent,
                                              _window,
                                              cx| {
                                            if !playable {
                                                this.notice =
                                                    Some("that one is not digitized yet".into());
                                            } else if this.snapshot.as_ref().is_some_and(|snapshot| {
                                                snapshot.queue.len() > index + 1
                                            }) {
                                                // Keep the rest of the queue: jump to
                                                // this entry instead of replacing it
                                                // with one track.
                                                this.request(Request::PlayFrom(index));
                                            } else {
                                                this.request(Request::Play(target.clone()));
                                            }
                                            cx.notify();
                                        },
                                    ),
                                )
                                .child(
                                    div()
                                        .w(px(18.0))
                                        .text_size(px(9.0))
                                        .text_color(rgb(if current_row { ACCENT } else { FAINT }))
                                        .child(format!("{:02}", index + 1)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .overflow_hidden()
                                        .text_size(px(10.0))
                                        .text_color(rgb(if current_row { TEXT } else { MUTED }))
                                        .child(SharedString::from(track.title.clone())),
                                )
                        }),
                ),
        );
        panel.into_any_element()
    }

    /// The cover for a track id, when one has been fetched.
    pub fn art_for(&self, id: &str) -> Option<Arc<Image>> {
        self.art.get(id).cloned()
    }

    pub fn set_art(&mut self, id: String, image: Arc<Image>) {
        self.art.insert(id, image);
    }

    /// The disc: the album's cover when the catalog could name one, else the
    /// vinyl drawing this view has always had.
    fn disc(&self, current: Option<&Track>) -> AnyElement {
        if let Some(track) = current
            && let Some(image) = self.art_for(&track.id)
        {
            return div()
                .flex_none()
                .h(px(210.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(PANEL))
                .child(
                    img(image)
                        .w(px(164.0))
                        .h(px(164.0))
                        .rounded(px(10.0))
                        .object_fit(ObjectFit::Cover),
                )
                .into_any_element();
        }
        let label = current
            .map(|track| track.title.chars().take(1).collect::<String>())
            .unwrap_or_else(|| "·".into());
        div()
            .flex_none()
            .h(px(210.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(PANEL))
            .child(
                div()
                    .w(px(164.0))
                    .h(px(164.0))
                    .rounded_full()
                    .bg(rgb(0x0a0908))
                    .border_1()
                    .border_color(rgb(0x1f1b15))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(62.0))
                            .h(px(62.0))
                            .rounded_full()
                            .bg(rgb(ACCENT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(22.0))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0x1a1409))
                                    .child(SharedString::from(label)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn transport_button(
        &self,
        id: &'static str,
        glyph: &'static str,
        cx: &mut Context<Self>,
        request: Request,
    ) -> AnyElement {
        div()
            .id(id)
            .w(px(34.0))
            .h(px(26.0))
            .rounded(px(4.0))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.0))
            .text_color(rgb(TEXT))
            .bg(rgb(PANEL_HI))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _: &gpui::MouseDownEvent, _window, cx| {
                    this.request(request.clone());
                    cx.notify();
                }),
            )
            .child(glyph)
            .into_any_element()
    }

    fn footer(&self) -> impl IntoElement {
        div()
            .h(px(FOOTER_H))
            .flex_none()
            .px(px(14.0))
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(PANEL))
            .border_t_1()
            .border_color(rgb(LINE))
            .child(div().text_size(px(9.0)).text_color(rgb(FAINT)).child(
                "/ search   enter play   a enqueue   space toggle   n/p skip   s source   q quit",
            ))
            .child(
                div()
                    .text_size(px(9.0))
                    .text_color(rgb(if self.error.is_some() { BAD } else { MUTED }))
                    .child(match (&self.error, &self.notice) {
                        (Some(error), _) => error.clone(),
                        (None, Some(notice)) => notice.clone(),
                        (None, None) => SharedString::from(match self.fetched_at {
                            Some(at) => format!(
                                "{} · {} results · {:.0}s ago",
                                self.side.label(),
                                self.results.len(),
                                at.elapsed().as_secs_f32()
                            ),
                            None => String::new(),
                        }),
                    }),
            )
    }
}

impl Render for Deck {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The owner keeps the truth; the thumb follows it unless a pointer is on
        // it. Doing this here means every snapshot path (event, poll, refresh)
        // updates the control without each of them knowing it exists.
        self.sync_volume(window, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(BG))
            .font_family("Lilex")
            .child(self.header(cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .overflow_hidden()
                    .child(self.library(cx))
                    .child(self.now(cx)),
            )
            .child(self.footer())
    }
}

pub fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    let (minutes, seconds) = (seconds / 60, seconds % 60);
    if minutes >= 60 {
        format!("{}:{:02}:{:02}", minutes / 60, minutes % 60, seconds)
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// `deadwax serve` answers `/search` with `TrackHit` objects. The CID is the
/// identity when there is one; a discogs row with no file yet is still listed,
/// because the hole is what a person may want to fill.
pub fn parse_catalog(body: &[u8]) -> Result<Vec<Track>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| format!("catalog is not JSON: {error}"))?;
    let list = value
        .as_array()
        .ok_or_else(|| "catalog response is not a list".to_string())?;
    let mut tracks = Vec::with_capacity(list.len());
    for entry in list {
        let text = |key: &str| {
            entry
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if text("kind") == "sample" {
            continue;
        }
        let cid = text("cid");
        tracks.push(Track {
            side: Side::Local,
            playable: !cid.is_empty(),
            id: cid,
            title: text("title"),
            artist: text("artist"),
            album: Some(text("album")).filter(|album| !album.is_empty()),
            duration_ms: None,
            art: None,
        });
    }
    Ok(tracks)
}

/// Parse `/playlists?coverage=1`. Coverage is optional in the response, so a
/// server that answers without it still lists the playlists — they just cannot
/// claim how much of each is owned.
pub fn parse_playlists(body: &[u8]) -> Result<Vec<Playlist>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| format!("playlists are not JSON: {error}"))?;
    let list = value
        .as_array()
        .ok_or_else(|| "playlists response is not a list".to_string())?;
    let mut playlists = Vec::with_capacity(list.len());
    for entry in list {
        let owner = entry
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let Some(list) = entry.get("playlist") else {
            continue;
        };
        let name = list
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let coverage = entry.get("coverage");
        let count = |key: &str| {
            coverage
                .and_then(|coverage| coverage.get(key))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize
        };
        let albums = coverage
            .and_then(|coverage| coverage.get("missing_albums"))
            .and_then(serde_json::Value::as_array)
            .map(|albums| {
                albums
                    .iter()
                    .filter_map(|album| {
                        Some(MissingAlbum {
                            artist: album.get("artist")?.as_str()?.to_string(),
                            album: album.get("album")?.as_str()?.to_string(),
                            missing: album
                                .get("wanted")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0)
                                .saturating_sub(
                                    album
                                        .get("owned")
                                        .and_then(serde_json::Value::as_u64)
                                        .unwrap_or(0),
                                ) as usize,
                        })
                    })
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
        playlists.push(Playlist {
            owner,
            name,
            tracks: count("tracks"),
            owned: count("owned"),
            missing: albums,
        });
    }
    Ok(playlists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_catalog_row_keeps_its_identity() {
        let body = br#"[{"id":7,"release_id":null,"source":"digital","kind":"music",
            "path":"/m/Adele/21/Rolling in the Deep.flac","album":"21",
            "artist":"Adele","position":"1","title":"Rolling in the Deep",
            "year":2011,"state":null,"label":null,"country":null,
            "cid":"bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku"}]"#;
        let tracks = parse_catalog(body).expect("a real response parses");
        assert_eq!(tracks.len(), 1);
        assert!(tracks[0].playable);
        assert_eq!(tracks[0].title, "Rolling in the Deep");
        assert!(tracks[0].id.starts_with("bafkrei"));
    }

    #[test]
    fn a_discogs_row_without_a_master_is_listed_but_not_playable() {
        let body = br#"[{"kind":"music","title":"In Bloom","artist":"Nirvana",
            "album":"Nevermind","cid":null}]"#;
        let tracks = parse_catalog(body).expect("a minimal row parses");
        assert_eq!(tracks.len(), 1);
        assert!(!tracks[0].playable, "no CID, no queue entry");
    }

    #[test]
    fn samples_are_not_the_music_list() {
        let body = br#"[{"kind":"sample","title":"Sandstorm stab","artist":"Darude",
            "album":"Before the Storm","cid":null}]"#;
        assert!(parse_catalog(body).unwrap().is_empty());
    }

    #[test]
    fn the_clock_is_a_duration_not_a_date() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(42_000), "0:42");
        assert_eq!(clock(366_000), "6:06");
        assert_eq!(clock(3_723_000), "1:02:03");
    }

    #[test]
    fn progress_is_bounded_even_when_the_owner_is_wrong() {
        let mut deck = Deck::new();
        deck.apply_snapshot(Snapshot {
            revision: 1,
            side: Side::Local,
            playing: true,
            position_ms: 500_000,
            volume: Some(0.5),
            repeat: RepeatMode::Off,
            shuffle: false,
            current: Some(Track {
                side: Side::Local,
                id: "bafkrei".into(),
                title: "t".into(),
                artist: "a".into(),
                album: None,
                duration_ms: Some(100_000),
                art: None,
                playable: true,
            }),
            index: None,
            queue: Vec::new(),
        });
        assert_eq!(deck.progress(), 1.0);

        deck.apply_progress(2, 50_000, false);
        assert_eq!(deck.position_ms(), 50_000);
        assert_eq!(deck.progress(), 0.5);

        // A late event from an older revision cannot move the playhead back.
        deck.apply_progress(1, 90_000, true);
        assert_eq!(deck.position_ms(), 50_000);
    }
}
