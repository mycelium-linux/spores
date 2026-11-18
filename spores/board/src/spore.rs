//! The GPUI lane: a Marbles board drawn offscreen and handed to the presenter.
//!
//! This module only exists for `wasm32`; the projections it draws live in
//! [`crate::model`] and are tested on the host.

wit_bindgen::generate!({
    path: "wit",
    world: "runner",
    with: {
        "wasi:webgpu/webgpu@0.3.0-rc.2": wgpu_wasi_webgpu::wasi::webgpu::webgpu,
    },
    generate_all,
});

use std::{borrow::Cow, sync::Arc};

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_base::Selectable;
use gpui_component::{Sizable, button::Button};
use gpui_wgpu::{CosmicTextSystem, WgpuContext, WgpuHeadlessRenderer};
use soil::present::{cursor, input, surface};
use soil::records::records;

use crate::model::{self, Burndown, Column, Issue};

const LILEX: &[u8] = include_bytes!("../assets/Lilex-Regular.ttf");
const LILEX_BOLD: &[u8] = include_bytes!("../assets/Lilex-Bold.ttf");

/// The records collection a Marbles store keeps. One collection holds every
/// record type; the board reads the `issue/` prefix and ignores the rest.
const COLLECTION: &str = "marbles";
/// Records per `scan`. A board is a view of a store that fits in one book; a
/// page this size keeps a few thousand issues to a handful of calls.
const PAGE: u32 = 512;
/// The window the burndown covers, in days.
const DAYS: i64 = 14;

const BG: u32 = 0x0d1117;
const PANEL: u32 = 0x131a22;
const EDGE: u32 = 0x232d38;
const INK: u32 = 0xe6edf5;
const MUTED: u32 = 0x8b98a6;
const DIM: u32 = 0x5d6b7a;
const ACCENT: u32 = 0x6ea8fe;
const GOOD: u32 = 0x7ad0a3;
const WARN: u32 = 0xffd166;
const BAD: u32 = 0xff7b72;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Board,
    Burndown,
}

/// Read every issue in the book, following the cursor to the end.
///
/// A failure is returned as text rather than panicking: a board that cannot
/// read its store has to say so on the frame, because the person looking at it
/// cannot see a stderr.
fn load() -> (Vec<Issue>, Option<String>) {
    let mut rows: Vec<(String, Vec<u8>)> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        match records::scan(COLLECTION, cursor.as_deref(), PAGE) {
            Ok(page) => {
                for entry in page.entries {
                    if entry.deleted {
                        continue;
                    }
                    rows.push((entry.key.id, entry.body));
                }
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            Err(error) => return (model::issues(&rows), Some(format!("{error:?}"))),
        }
    }
    (model::issues(&rows), None)
}

pub struct Board {
    issues: Vec<Issue>,
    query: String,
    view: View,
    error: Option<SharedString>,
}

impl Board {
    fn new() -> Self {
        let (issues, error) = load();
        Self {
            issues,
            query: String::new(),
            view: View::Board,
            error: error.map(SharedString::from),
        }
    }

    fn reload(&mut self) {
        let (issues, error) = load();
        self.issues = issues;
        self.error = error.map(SharedString::from);
    }

    /// How many issues the query admits.
    ///
    /// Derived, never stored: a count kept beside the data is a count that can
    /// disagree with it, and the header is the first place anyone would notice.
    fn shown(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| model::matches(issue, &self.query))
            .count()
    }

    fn filtered(&self) -> Vec<Issue> {
        self.issues
            .iter()
            .filter(|issue| model::matches(issue, &self.query))
            .cloned()
            .collect()
    }

    /// What a key means to a board.
    ///
    /// `Some(changed)` means keep running and say whether the frame changed;
    /// `None` means the viewer asked to leave. Escape is the two-step every
    /// other GPUI Spore has: it clears the search first, and only then quits —
    /// so a stray escape does not lose the window.
    fn handle_key(&mut self, key: &input::Key) -> Option<bool> {
        if !key.pressed {
            return Some(false);
        }
        match key.key.as_str() {
            "escape" => {
                if self.query.is_empty() {
                    return None;
                }
                self.query.clear();
                Some(true)
            }
            "backspace" => Some(self.query.pop().is_some()),
            "tab" => {
                self.view = match self.view {
                    View::Board => View::Burndown,
                    View::Burndown => View::Board,
                };
                Some(true)
            }
            // A control chord is not text. Without this, ⌘R would type an `r`
            // into the search box as well as reloading.
            _ if key.control || key.meta || key.alt => {
                if key.key == "r" {
                    self.reload();
                    return Some(true);
                }
                Some(false)
            }
            _ => match key.text.as_ref() {
                Some(text) if !text.is_empty() => {
                    self.query.push_str(text);
                    Some(true)
                }
                _ => Some(false),
            },
        }
    }

    fn header(&self) -> AnyElement {
        let scheme = match self.view {
            View::Board => "Board",
            View::Burndown => "Burndown",
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(58.0))
            .px(px(22.0))
            .flex_shrink_0()
            .bg(rgb(PANEL))
            .border_b_1()
            .border_color(rgb(EDGE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK))
                            .child("board"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(DIM))
                            .child(SharedString::from(format!("marbles · {scheme}"))),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(MUTED))
                    .child(SharedString::from(format!(
                        "{} issues · {} shown",
                        self.issues.len(),
                        self.shown()
                    ))),
            )
            .into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = |label: &'static str, target: View, current: View, cx: &mut Context<Self>| {
            let picked = target == current;
            Button::new(ElementId::Name(format!("tab-{label}").into()))
                .label(label)
                .small()
                .when(picked, |button| button.selected(true))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.view = target;
                    cx.notify();
                }))
                .into_any_element()
        };

        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .h(px(46.0))
            .px(px(22.0))
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(EDGE))
            .child(tab("Board", View::Board, self.view, cx))
            .child(tab("Burndown", View::Burndown, self.view, cx))
            .child(
                // The search box is drawn, not a native input: a Spore receives
                // normalized keys, so the field is the query plus a caret.
                div()
                    .flex()
                    .items_center()
                    .flex_1()
                    .max_w(px(360.0))
                    .h(px(28.0))
                    .px(px(10.0))
                    .ml(px(10.0))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(EDGE))
                    .bg(rgb(BG))
                    .text_size(px(12.0))
                    .child(match self.query.is_empty() {
                        true => div().text_color(rgb(DIM)).child("search"),
                        false => div()
                            .text_color(rgb(INK))
                            .child(SharedString::from(self.query.clone())),
                    })
                    .child(div().w(px(1.0)).h(px(13.0)).ml(px(1.0)).bg(rgb(ACCENT))),
            )
            .into_any_element()
    }

    fn board(&self) -> AnyElement {
        let columns = model::columns(&self.issues, &self.query);
        let mut row = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(12.0))
            .p(px(16.0))
            .flex_1()
            .overflow_hidden();
        for column in columns {
            row = row.child(card_column(&column));
        }
        row.into_any_element()
    }

    fn burndown(&self) -> AnyElement {
        let issues = self.filtered();
        let now = model::latest(&self.issues);
        let burn = model::burndown(&issues, DAYS, now);

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(16.0))
            .flex_1()
            .child(stats(&burn))
            .child(chart(&burn))
            .child(days(&burn))
            .into_any_element()
    }
}

fn priority_color(priority: i64) -> u32 {
    match priority {
        0 => BAD,
        1 => WARN,
        2 => ACCENT,
        _ => DIM,
    }
}

/// One status column: a heading with its count, then the cards.
fn card_column(column: &Column) -> AnyElement {
    // A scrolling element is stateful in GPUI, so it needs an id: the scroll
    // offset is per-element state, and two columns sharing one would scroll
    // together.
    let mut body = div()
        .id(ElementId::Name(format!("column/{}", column.status).into()))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(10.0))
        .flex_1()
        .overflow_y_scroll();
    for card in &column.cards {
        body = body.child(card_view(card));
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(150.0))
        .h_full()
        .rounded_lg()
        .border_1()
        .border_color(rgb(EDGE))
        .bg(rgb(PANEL))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(12.0))
                .h(px(34.0))
                .flex_shrink_0()
                .border_b_1()
                .border_color(rgb(EDGE))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(INK))
                        .child(column.title),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(DIM))
                        .child(SharedString::from(column.count().to_string())),
                ),
        )
        .child(body)
        .into_any_element()
}

/// One card: the short id, the title, and what it is worth.
fn card_view(card: &model::Card) -> AnyElement {
    let mut row = div().flex().items_center().gap(px(6.0));
    row = row.child(
        div()
            .text_size(px(10.0))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(priority_color(card.priority)))
            .child(SharedString::from(format!("P{}", card.priority))),
    );
    row = row.child(
        div()
            .text_size(px(10.0))
            .text_color(rgb(DIM))
            .child(SharedString::from(card.id.clone())),
    );
    if !card.labels.is_empty() {
        row = row.child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(MUTED))
                .child(SharedString::from(format!("· {}", card.labels.join(" ")))),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .p(px(9.0))
        .rounded_md()
        .border_1()
        .border_color(rgb(EDGE))
        .bg(rgb(BG))
        .child(row)
        .child(
            div()
                .text_size(px(12.0))
                .text_color(rgb(INK))
                .line_clamp(3)
                .child(SharedString::from(card.title.clone())),
        )
        .into_any_element()
}

fn stat(label: &'static str, value: String, ink: u32) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .flex_1()
        .child(div().text_size(px(10.0)).text_color(rgb(DIM)).child(label))
        .child(
            div()
                .text_size(px(18.0))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(ink))
                .child(SharedString::from(value)),
        )
        .into_any_element()
}

fn stats(burn: &Burndown) -> AnyElement {
    let projected = burn
        .projected_days_to_zero
        .map(|days| format!("{days:.1}d"))
        .unwrap_or_else(|| "—".to_string());
    let rate = burn
        .burn_per_day
        .map(|rate| format!("{rate:.2}/d"))
        .unwrap_or_else(|| "—".to_string());

    div()
        .flex()
        .items_center()
        .gap(px(18.0))
        .p(px(14.0))
        .rounded_lg()
        .border_1()
        .border_color(rgb(EDGE))
        .bg(rgb(PANEL))
        .child(stat("scope", burn.scope.to_string(), INK))
        .child(stat("done", burn.done.to_string(), GOOD))
        .child(stat("remaining", burn.remaining.to_string(), ACCENT))
        .child(stat("burn", rate, MUTED))
        .child(stat("projected", projected, WARN))
        .into_any_element()
}

/// The chart itself: an ideal line and the actual remaining line.
fn chart(burn: &Burndown) -> AnyElement {
    let days = burn.days.clone();
    let peak = burn.peak() as f32;

    div()
        .h(px(300.0))
        .w_full()
        .rounded_lg()
        .border_1()
        .border_color(rgb(EDGE))
        .bg(rgb(PANEL))
        .overflow_hidden()
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let n = days.len().max(2);
                    let x_at = |index: usize| {
                        bounds.origin.x + bounds.size.width * (index as f32 / (n - 1) as f32)
                    };
                    let y_at = |value: i64| {
                        let value = (value as f32).clamp(0.0, peak);
                        bounds.origin.y + bounds.size.height * (1.0 - value / peak)
                    };

                    // The ideal line: where an even burn would have finished.
                    let mut ideal = PathBuilder::stroke(px(1.5)).dash_array(&[px(5.0), px(4.0)]);
                    for (index, day) in days.iter().enumerate() {
                        let at = point(x_at(index), y_at(day.ideal));
                        if index == 0 {
                            ideal.move_to(at);
                        } else {
                            ideal.line_to(at);
                        }
                    }
                    if let Ok(path) = ideal.build() {
                        window.paint_path(path, rgb(EDGE));
                    }

                    // The actual line.
                    let mut actual = PathBuilder::stroke(px(2.0));
                    for (index, day) in days.iter().enumerate() {
                        let at = point(x_at(index), y_at(day.remaining));
                        if index == 0 {
                            actual.move_to(at);
                        } else {
                            actual.line_to(at);
                        }
                    }
                    if let Ok(path) = actual.build() {
                        window.paint_path(path, rgb(ACCENT));
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

/// The window's ends, so the chart has a scale without painting text.
fn days(burn: &Burndown) -> AnyElement {
    let first = burn.days.first().map(|day| day.at).unwrap_or(0);
    let last = burn.days.last().map(|day| day.at).unwrap_or(0);
    div()
        .flex()
        .items_center()
        .justify_between()
        .text_size(px(10.0))
        .text_color(rgb(DIM))
        .child(SharedString::from(model::date_label(first)))
        .child(SharedString::from(model::date_label(last)))
        .into_any_element()
}

impl Render for Board {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.view {
            View::Board => self.board(),
            View::Burndown => self.burndown(),
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .font_family("Lilex")
            .text_size(px(13.0))
            .text_color(rgb(INK))
            .child(self.header())
            .child(self.toolbar(cx))
            .child(body)
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .px(px(22.0))
                        .py(px(8.0))
                        .text_size(px(11.0))
                        .text_color(rgb(BAD))
                        .child(SharedString::from(format!("records: {error}"))),
                )
            })
    }
}

fn present_frame(
    context: &WgpuContext,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> anyhow::Result<()> {
    let texture = context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("board frame"),
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

fn run_board() -> anyhow::Result<()> {
    let (width, height) = surface::size();
    let instance = wgpu_wasi_webgpu::instance();
    let adapter = wit_bindgen::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = wit_bindgen::block_on(adapter.request_device(&Default::default()))?;
    let (context, format) = WgpuContext::from_offscreen(instance, adapter, device, queue)?;

    // A Spore carries its own fonts: the guest has no system to ask, and
    // shaping with no face traps in the text system.
    let text = Arc::new(CosmicTextSystem::new_without_system_fonts("Lilex"));
    text.add_fonts(vec![Cow::Borrowed(LILEX), Cow::Borrowed(LILEX_BOLD)])?;

    let factory = context.clone();
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(()), move || {
        Ok(Some(Box::new(WgpuHeadlessRenderer::with_context(
            factory.clone(),
            format,
        )?)))
    });

    // gpui-component reads globals — the theme, inputs, popovers — while
    // rendering, so they must exist before the first draw. `HeadlessAppContext`
    // hands out `&mut App` only through `update`; `as_mut` panics by design.
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
    });

    let scale = surface::scale() as f32;
    let window =
        cx.open_window_scaled(size(px(width as f32), px(height as f32)), scale, |_, cx| {
            cx.new(|_| Board::new())
        })?;
    let handle: AnyWindowHandle = window.into();

    // One context, one window, as many frames as the host asks for. A capture
    // host is not interactive: it draws once and the loop ends. A live
    // presenter keeps the window, so a redraw costs a draw rather than a
    // device, a font stack and a window.
    let mut dirty = true;
    let mut extent = (width, height);
    loop {
        let now = surface::size();
        if now != extent {
            extent = now;
            window.update(&mut cx, |_, window, _| {
                window.resize(size(px(now.0 as f32), px(now.1 as f32)));
            })?;
            dirty = true;
        }

        if dirty {
            cx.update_window(handle, |_, window, app| {
                let _ = window.draw(app);
            })?;
            let image = cx.capture_screenshot(handle)?;
            present_frame(&context, image.width(), image.height(), image.as_raw())?;
            dirty = false;
        }

        if !input::interactive() {
            break;
        }

        match input::poll(16) {
            Some(input::Event::KeyInput(key)) => {
                let changed = window.update(&mut cx, |board, _, cx| {
                    let changed = board.handle_key(&key);
                    if changed == Some(true) {
                        cx.notify();
                    }
                    changed
                })?;
                match changed {
                    Some(changed) => dirty |= changed,
                    // The viewer asked to leave.
                    None => break,
                }
            }
            // A pointer is an event like any other: the host normalizes it and
            // the view decides what is under it.
            Some(input::Event::Moved(moved)) => {
                cx.update_window(handle, |_, window, app| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                            position: point(px(moved.x), px(moved.y)),
                            pressed_button: None,
                            modifiers: gpui::Modifiers::default(),
                        }),
                        app,
                    );
                })?;
                dirty = true;
            }
            Some(input::Event::Button(button)) => {
                let mouse = match button.button {
                    input::PointerButton::Left => gpui::MouseButton::Left,
                    input::PointerButton::Right => gpui::MouseButton::Right,
                    input::PointerButton::Middle => gpui::MouseButton::Middle,
                };
                let position = point(px(button.x), px(button.y));
                cx.update_window(handle, |_, window, app| {
                    window.dispatch_event(
                        gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                            position,
                            pressed_button: button.pressed.then_some(mouse),
                            modifiers: gpui::Modifiers::default(),
                        }),
                        app,
                    );
                    if button.pressed {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                                button: mouse,
                                position,
                                modifiers: gpui::Modifiers::default(),
                                click_count: 1,
                                first_mouse: false,
                            }),
                            app,
                        );
                    } else {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                                button: mouse,
                                position,
                                modifiers: gpui::Modifiers::default(),
                                click_count: 1,
                            }),
                            app,
                        );
                    }
                })?;
                dirty = true;
            }
            Some(input::Event::Scrolled(scrolled)) => {
                cx.update_window(handle, |_, window, app| {
                    window.dispatch_event(
                        gpui::PlatformInput::Scroll(gpui::ScrollEvent {
                            position: point(px(scrolled.x), px(scrolled.y)),
                            delta: gpui::ScrollDelta::Lines(point(0., scrolled.lines)),
                            ..Default::default()
                        }),
                        app,
                    );
                })?;
                dirty = true;
            }
            None => {}
        }
    }

    let _ = cursor::set(cursor::CursorStyle::Default);
    Ok(())
}

struct Component;

impl Guest for Component {
    async fn run() -> u32 {
        match run_board() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("board: {error:#}");
                1
            }
        }
    }
}

export!(Component);
