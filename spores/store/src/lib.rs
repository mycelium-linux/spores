//! Store view Spore: GPUI lays out the shopfront, `wasi:webgpu` renders it, and
//! `soil:canvas-gpu` presents the texture to Canvas.
//!
//! The guest never gets a window or a swapchain, and never opens a socket: the
//! catalog is read with `soil:http`, which the host checks against its origin
//! allowlist. What the Spore owns is the shopfront — the account chip, the
//! cards, the states a client can be in. Canvas owns the window and the input.
//!
//! The same package also ships a `web` surface (`web/index.html`) for a browser
//! that has no Canvas; both read `/api/catalog` from the same store service, so
//! there is one catalog and two ways to draw it.

wit_bindgen::generate!({
    path: "wit",
    world: "runner",
    with: {
        "wasi:webgpu/webgpu@0.3.0-rc.2": wgpu_wasi_webgpu::wasi::webgpu::webgpu,
    },
    generate_all,
});

use std::borrow::Cow;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{AnyElement, size,
    div, px, rgb, AnyWindowHandle, Context, FontWeight, HeadlessAppContext, IntoElement,
    ParentElement, PlatformTextSystem as _, Render, SharedString, Styled, Window,
};
use gpui_wgpu::{CosmicTextSystem, WgpuContext, WgpuHeadlessRenderer};
use soil::http::http;
use soil::present::{input, surface};
use soil::spore::catalog;

/// IBM Plex Sans, SIL Open Font License 1.1. A wasm component has no system
/// fonts, so the Spore ships the one face its view names.
const FONT: &[u8] = include_bytes!("../assets/IBMPlexSans-Regular.ttf");
const FONT_FAMILY: &str = "IBM Plex Sans";

/// The store this Spore reads. The host decides whether the origin is allowed;
/// a fleet store points this at its own index by repackaging with a different
/// value, which is a package revision, not a setting the guest can flip.
const ORIGIN: &str = "http://127.0.0.1:8800";

const BG: u32 = 0x0b0d12;
const PANEL: u32 = 0x141821;
const PANEL_RAISED: u32 = 0x1a2233;
const EDGE: u32 = 0x222735;
const TEXT: u32 = 0xe9ecf3;
const MUTED: u32 = 0x98a1b3;
const ACCENT: u32 = 0x8b9dff;
const GOOD: u32 = 0x7ad0a3;
const WARN: u32 = 0xf2c14e;

struct Component;
export!(Component);

impl Guest for Component {
    async fn run() -> u32 {
        if let Err(error) = render() {
            eprintln!("store-view: {error}");
            return 1;
        }
        // A Spore that returns closes its window, and a storefront is a thing
        // you keep looking at. Stay up until the presenter goes away or someone
        // asks to close. Interaction (tabs, buttons) lands on top of this loop.
        loop {
            if !input::interactive() {
                return 0;
            }
            match input::poll(250) {
                Some(input::Event::KeyInput(key)) if key.key == "escape" => return 0,
                _ => {}
            }
        }
    }
}

/// The index the store service served: its name when one answered, whether
/// its signature checked out, and how many packages it carries.
#[derive(Clone)]
struct Source {
    name: Option<String>,
    verified: bool,
    total: usize,
}

/// A package's own mark: the badge and colour its manifest declares for a
/// launcher row, and later the artwork a listing points at. The store draws
/// what the package says about itself rather than inventing a colour for it.
#[derive(Clone)]
struct Mark {
    badge: String,
    color: Option<u32>,
}

impl Mark {
    fn of(title: &str, badge: &str, color: Option<u32>) -> Self {
        let badge = if badge.is_empty() || badge == "?" {
            title
                .chars()
                .next()
                .map(|value| value.to_uppercase().to_string())
                .unwrap_or_else(|| "?".to_string())
        } else {
            badge.to_string()
        };
        Self { badge, color }
    }
}

#[derive(Clone)]
struct Row {
    /// What a person reads. A listing's name when the index has one, the
    /// installed package's own title when it does not, and the id's last
    /// segment as the last resort.
    title: String,
    id: String,
    summary: String,
    description: String,
    version: String,
    installed_version: Option<String>,
    state: String,
    mark: Mark,
    /// What applying the offering would cost, when the store has planned it:
    /// `live` applies now, `restart` at the next launch, `reboot` after one.
    change: Option<Change>,
}

#[derive(Clone)]
struct Change {
    class: String,
    reason: String,
}

/// One GET against the store service. The host owns the origin policy: a
/// refusal or a transport failure is `None`, not an empty answer.
fn get(path: &str) -> Option<serde_json::Value> {
    let request = http::Request {
        url: format!("{ORIGIN}{path}"),
        method: http::Method::Get,
        headers: vec![http::Header {
            name: "accept".to_string(),
            value: "application/json".to_string(),
        }],
        body: None,
        credential: None,
        max_bytes: 1 << 20,
        timeout_ms: 4_000,
    };
    let response = http::fetch(&request).ok()?;
    if response.status != 200 {
        return None;
    }
    serde_json::from_slice(&response.body).ok()
}

/// What the store host knows about a package it has on disk. The catalog rows
/// describe what is for sale; this describes what the package says about
/// itself, which is the same answer for a row and for the machine list.
#[derive(Default)]
struct Facts {
    title: String,
    summary: String,
    description: String,
    badge: String,
    color: Option<u32>,
}

fn facts_of() -> std::collections::BTreeMap<String, Facts> {
    let mut facts = std::collections::BTreeMap::new();
    let Some(apps) = get("/api/apps") else {
        return facts;
    };
    for app in apps["apps"].as_array().cloned().unwrap_or_default() {
        let id = app["id"].as_str().unwrap_or_default().to_string();
        if id.is_empty() {
            continue;
        }
        facts.insert(
            id,
            Facts {
                title: title_of(&app["name"].as_str().unwrap_or_default().to_string()),
                summary: app["summary"].as_str().unwrap_or_default().to_string(),
                description: app["description"].as_str().unwrap_or_default().to_string(),
                badge: app["badge"].as_str().unwrap_or_default().to_string(),
                color: parse_color(app["color"].as_str().unwrap_or_default()),
            },
        );
    }
    facts
}

/// The store's own plan, which is the closest thing to release notes this
/// system has: it says what changed and what applying it costs.
fn plan_of() -> std::collections::BTreeMap<String, Change> {
    let mut plan = std::collections::BTreeMap::new();
    let Some(value) = get("/api/plan") else {
        return plan;
    };
    for update in value["updates"].as_array().cloned().unwrap_or_default() {
        let id = update["id"].as_str().unwrap_or_default().to_string();
        if id.is_empty() {
            continue;
        }
        plan.insert(
            id,
            Change {
                class: update["class"].as_str().unwrap_or_default().to_string(),
                reason: update["reason"].as_str().unwrap_or_default().to_string(),
            },
        );
    }
    plan
}

fn catalog_rows() -> Result<(Vec<Row>, Source), String> {
    let installed = catalog::entries().map_err(|error| format!("catalog: {error:?}"))?;
    let facts = facts_of();
    let plan = plan_of();

    let mut source = Source {
        name: None,
        verified: false,
        total: 0,
    };
    let mut rows: Vec<Row> = Vec::new();
    let mut listed = std::collections::BTreeSet::new();
    if let Some(remote) = get("/api/catalog") {
        source.name = remote["source"]["name"].as_str().map(str::to_string);
        source.verified = remote["source"]["verified"].as_bool().unwrap_or(false);
        let entries = remote["entries"].as_array().cloned().unwrap_or_default();
        source.total = entries.len();
        for entry in entries {
            let id = entry["id"].as_str().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }
            let document = &entry["document"];
            let local = facts.get(&id);
            // Until the index carries a listing, the package installed on this
            // machine is the only honest source of a name, a sentence and a
            // mark: it described itself when it was built. Nothing here is
            // invented, and an absent description stays absent.
            let title = document["name"]
                .as_str()
                .map(str::to_string)
                .or_else(|| local.map(|facts| facts.title.clone()))
                .unwrap_or_else(|| title_of(&id));
            rows.push(Row {
                mark: Mark::of(
                    &title,
                    local.map(|facts| facts.badge.as_str()).unwrap_or_default(),
                    local.and_then(|facts| facts.color),
                ),
                title,
                summary: document["summary"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| local.map(|facts| facts.summary.clone()))
                    .unwrap_or_default(),
                description: local
                    .map(|facts| facts.description.clone())
                    .unwrap_or_default(),
                version: entry["version"].as_str().unwrap_or_default().to_string(),
                installed_version: entry["installed_version"].as_str().map(str::to_string),
                state: entry["state"].as_str().unwrap_or("available").to_string(),
                change: plan.get(&id).map(|change| Change {
                    class: change.class.clone(),
                    reason: change.reason.clone(),
                }),
                id: id.clone(),
            });
            listed.insert(id);
        }
    }
    for entry in installed {
        if listed.contains(&entry.id) {
            continue;
        }
        let local = facts.get(&entry.id);
        let title = local
            .map(|facts| facts.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| title_of(&entry.id));
        rows.push(Row {
            mark: Mark::of(
                &title,
                local.map(|facts| facts.badge.as_str()).unwrap_or_default(),
                local.and_then(|facts| facts.color),
            ),
            summary: entry.summary,
            description: local
                .map(|facts| facts.description.clone())
                .unwrap_or_default(),
            version: entry.version,
            installed_version: None,
            state: "installed".to_string(),
            change: None,
            title,
            id: entry.id,
        });
    }
    rows.sort_by_key(|row| row.title.to_lowercase());
    Ok((rows, source))
}

/// `#rrggbb` as the renderer wants it. A colour the manifest got wrong is the
/// same as no colour: the tile falls back instead of drawing something odd.
fn parse_color(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// Who is asking. A loopback request is the machine itself; a request through
/// the Mycelium gateway names the principal it authenticated.
#[derive(Clone)]
struct Account {
    name: String,
    note: String,
}

fn account() -> Account {
    if let Some(value) = get("/api/whoami") {
        let principal = value["principal"].as_str().unwrap_or("local");
        return Account {
            name: principal
                .strip_prefix("mycelium:")
                .unwrap_or(principal)
                .to_string(),
            note: if value["through_gateway"].as_bool().unwrap_or(false) {
                "signed in"
            } else {
                "this machine"
            }
            .to_string(),
        };
    }
    Account {
        name: "local".to_string(),
        note: "this machine".to_string(),
    }
}

// ── the shopfront ───────────────────────────────────────────────────────────
//
// Two rules shape this view, both borrowed from `weeper`, which is the most
// advanced UI this tree has:
//
//   one context, one window, as many frames as the host asks for — a redraw
//   costs a draw, not a device, a font stack and a window; and
//
//   every input is an event the view answers, so hit-testing and layout share
//   one set of numbers instead of two that drift.
//
// A tab is a rectangle and a row is a rectangle; the pointer is compared against
// the same constants that drew them.

const MAX_COLUMN: f32 = 980.0;
const PAGE_PAD: f32 = 28.0;
const HEADER_H: f32 = 54.0;
const TABS_H: f32 = 38.0;
const FOOTER_H: f32 = 30.0;
const HERO_H: f32 = 168.0;
const HERO_GAP: f32 = 14.0;
const ROW_H: f32 = 66.0;
const ROW_GAP: f32 = 6.0;
const TILE: f32 = 42.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Discover,
    Installs,
    Updates,
    Settings,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Discover, Tab::Installs, Tab::Updates, Tab::Settings];
    fn label(self) -> &'static str {
        match self {
            Tab::Discover => "Discover",
            Tab::Installs => "Installed",
            Tab::Updates => "Updates",
            Tab::Settings => "Settings",
        }
    }
}

/// What a pointer landed on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Tab(Tab),
    Row(usize),
    Action(usize),
    Refresh,
}

struct StoreView {
    rows: Vec<Row>,
    source: Source,
    account: Account,
    tab: Tab,
    /// The last thing the store did, in the store's own words.
    message: String,
    /// The row under the pointer, so a click reads as aimed.
    hovered: Option<usize>,
    width: f32,
    height: f32,
}

impl StoreView {
    fn new(rows: Vec<Row>, source: Source, account: Account, width: f32, height: f32) -> Self {
        let message = if rows.is_empty() {
            match &source.name {
                Some(name) => format!("{name} has nothing to offer this machine yet"),
                None => "No store reached, and nothing is installed here.".to_string(),
            }
        } else {
            format!("{} listings · {} installed", source.total, rows.iter().filter(|row| row.state == "installed").count())
        };
        Self {
            rows,
            source,
            account,
            tab: Tab::Discover,
            message,
            hovered: None,
            width,
            height,
        }
    }

    fn column(&self) -> (f32, f32) {
        let width = self.width.min(MAX_COLUMN);
        let margin = ((self.width - width) / 2.0).max(PAGE_PAD);
        (width - 2.0 * PAGE_PAD, margin + PAGE_PAD)
    }

    fn rows_on(&self, tab: Tab) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match tab {
                Tab::Installs => row.state == "installed" || row.state == "update",
                Tab::Updates => row.state == "update",
                _ => true,
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// What a tab offers, as a badge: the count a person actually cares about.
    fn badge(&self, tab: Tab) -> Option<usize> {
        match tab {
            Tab::Discover => None,
            Tab::Installs => Some(self.rows.iter().filter(|row| row.state != "available").count()),
            Tab::Updates => Some(self.rows.iter().filter(|row| row.state == "update").count()),
            Tab::Settings => None,
        }
    }

    fn tab_at(&self, x: f32, y: f32) -> Option<Tab> {
        let (_, left) = self.column();
        let top = HEADER_H;
        if y < top || y > top + TABS_H {
            return None;
        }
        let mut cursor = left;
        for tab in Tab::ALL {
            let width = 26.0 + 8.0 * tab.label().len() as f32;
            if x >= cursor && x <= cursor + width {
                return Some(tab);
            }
            cursor += width + 10.0;
        }
        None
    }

    /// Which row a point is on, and whether it is over that row's button.
    fn row_at(&self, x: f32, y: f32) -> Option<(usize, bool)> {
        let (width, left) = self.column();
        let mut top = HEADER_H + TABS_H + 46.0;
        if self.tab == Tab::Discover {
            top += HERO_H + 30.0 + 28.0;
        }
        let rows = self.rows_on(self.tab);
        for index in rows {
            if y >= top && y <= top + ROW_H {
                let button = x >= left + width - 132.0;
                return Some((index, button));
            }
            top += ROW_H + ROW_GAP;
        }
        None
    }

    fn target_at(&self, x: f32, y: f32) -> Option<Target> {
        if let Some(tab) = self.tab_at(x, y) {
            return Some(Target::Tab(tab));
        }
        let (width, left) = self.column();
        if self.tab == Tab::Settings
            && y >= HEADER_H + TABS_H + 120.0
            && y <= HEADER_H + TABS_H + 156.0
            && x >= left
            && x <= left + 150.0
        {
            return Some(Target::Refresh);
        }
        self.row_at(x, y)
            .map(|(index, button)| if button { Target::Action(index) } else { Target::Row(index) })
    }

    /// What a click means. Returns whether anything changed.
    fn activate(&mut self, target: Target) -> bool {
        match target {
            Target::Tab(tab) => {
                if self.tab == tab {
                    return false;
                }
                self.tab = tab;
                true
            }
            Target::Row(index) => {
                self.hovered = Some(index);
                true
            }
            Target::Action(index) => self.action(index),
            Target::Refresh => {
                self.reload();
                true
            }
        }
    }

    /// The one button a row offers, which depends on what that row is.
    fn action(&mut self, index: usize) -> bool {
        let Some(row) = self.rows.get(index) else {
            return false;
        };
        let id = row.id.clone();
        let title = row.title.clone();
        match row.state.as_str() {
            "available" | "update" => {
                self.message = format!("Installing {title}…");
                match catalog::install(&id) {
                    Ok(installed) => {
                        self.message = format!("Installed {title} {}", installed.version);
                        self.reload();
                    }
                    Err(failure) => {
                        self.message = format!("{title} did not install: {}", detail(&failure));
                    }
                }
            }
            _ => match catalog::launch(&id, "canvas") {
                Ok(launched) => self.message = format!("Opened {title} ({})", launched.target),
                Err(failure) => self.message = format!("{title} did not open: {}", detail(&failure)),
            },
        }
        true
    }

    fn reload(&mut self) {
        match catalog_rows() {
            Ok((rows, source)) => {
                self.source = source;
                self.rows = rows;
            }
            Err(error) => {
                self.message = format!("The store could not be read: {error}");
            }
        }
    }

    fn handle_key(&mut self, key: &str) -> bool {
        match key {
            "1" | "2" | "3" | "4" => {
                let index = key.parse::<usize>().unwrap_or(1) - 1;
                let tab = Tab::ALL[index];
                if self.tab == tab {
                    return false;
                }
                self.tab = tab;
                true
            }
            "arrow-left" | "h" => self.cycle(-1),
            "arrow-right" | "l" => self.cycle(1),
            "enter" => self
                .hovered
                .or_else(|| self.rows_on(self.tab).first().copied())
                .is_some_and(|index| self.action(index)),
            _ => false,
        }
    }

    fn cycle(&mut self, step: i32) -> bool {
        let at = Tab::ALL.iter().position(|tab| *tab == self.tab).unwrap_or(0) as i32;
        let next = (at + step).rem_euclid(Tab::ALL.len() as i32) as usize;
        self.tab = Tab::ALL[next];
        true
    }

    fn header(&self) -> impl IntoElement {
        let (_, left) = self.column();
        div()
            .h(px(HEADER_H))
            .flex()
            .items_center()
            .justify_between()
            .pl(px(left))
            .pr(px(left))
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
                            .child("Store"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(MUTED))
                            .child(match &self.source.name {
                                Some(name) if self.source.verified => format!("{name} · signed"),
                                Some(name) => format!("{name} · unverified"),
                                None => "no store reached".to_string(),
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(MUTED))
                            .child(format!("{} · {}", self.account.name, self.account.note)),
                    ),
            )
    }

    fn tab_strip(&self) -> impl IntoElement {
        let (_, left) = self.column();
        let mut strip = div()
            .h(px(TABS_H))
            .flex()
            .items_center()
            .gap(px(10.0))
            .pl(px(left))
            .pr(px(left));
        for tab in Tab::ALL {
            let selected = self.tab == tab;
            let mut label = div()
                .px(px(13.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .text_size(px(12.0))
                .text_color(if selected { rgb(TEXT) } else { rgb(MUTED) })
                .bg(if selected { rgb(PANEL_RAISED) } else { rgb(0x00000000) })
                .child(tab.label());
            if let Some(count) = self.badge(tab) {
                if count > 0 {
                    label = label.child(
                        div()
                            .pl(px(6.0))
                            .text_size(px(10.0))
                            .text_color(rgb(ACCENT))
                            .child(format!("{count}")),
                    );
                }
            }
            strip = strip.child(label);
        }
        strip
    }

    fn hero_card(&self, index: usize) -> AnyElement {
        let row = &self.rows[index];
        div()
            .w(px(300.0))
            .h(px(HERO_H))
            .flex()
            .flex_col()
            .justify_end()
            .rounded(px(14.0))
            .p(px(14.0))
            .bg(rgb(row.mark.color.unwrap_or(PANEL_RAISED)))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x0b0d12))
                    .child(row.title.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x1a2233))
                    .child(format!("{} · {}", row.version, truncate(&row.summary, 64))),
            )
            .into_any_element()
    }

    fn row_card(&self, index: usize) -> AnyElement {
        let row = &self.rows[index];
        let hovered = self.hovered == Some(index);
        let (width, _) = self.column();
        div()
            .w(px(width))
            .h(px(ROW_H))
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(12.0))
            .rounded(px(12.0))
            .bg(if hovered { rgb(PANEL_RAISED) } else { rgb(PANEL) })
            .child(
                div()
                    .w(px(TILE))
                    .h(px(TILE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(10.0))
                    .bg(rgb(row.mark.color.unwrap_or(PANEL_RAISED)))
                    .text_size(px(17.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x0b0d12))
                    .child(row.mark.badge.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(TEXT))
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(MUTED))
                            .child(truncate(&row.summary, 96)),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child(match row.state.as_str() {
                        "update" => format!(
                            "{} → {}",
                            row.installed_version.clone().unwrap_or_default(),
                            row.version
                        ),
                        "installed" => row.version.clone(),
                        _ => row.change.as_ref().map(|change| change.class.clone()).unwrap_or_default(),
                    }),
            )
            .child(self.action_button(row))
            .into_any_element()
    }

    fn action_button(&self, row: &Row) -> AnyElement {
        let label = match row.state.as_str() {
            "available" => "Get",
            "update" => "Update",
            _ => "Open",
        };
        div()
            .w(px(96.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .bg(rgb(if row.state == "available" || row.state == "update" {
                ACCENT
            } else {
                PANEL_RAISED
            }))
            .flex()
            .justify_center()
            .text_size(px(11.0))
            .text_color(rgb(if row.state == "available" || row.state == "update" {
                0x0b0d12
            } else {
                TEXT
            }))
            .child(label)
            .into_any_element()
    }

    fn body(&self) -> AnyElement {
        let (width, left) = self.column();
        let rows = self.rows_on(self.tab);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .pl(px(left))
            .pr(px(left))
            .pt(px(16.0));

        match self.tab {
            Tab::Discover => {
                let heroes: Vec<usize> = rows
                    .iter()
                    .copied()
                    .filter(|index| self.rows[*index].state != "installed")
                    .take(3)
                    .collect();
                if !heroes.is_empty() {
                    body = body.child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(TEXT))
                            .pb(px(10.0))
                            .child("New releases"),
                    );
                    let mut strip = div().flex().flex_row().gap(px(HERO_GAP));
                    for index in heroes {
                        strip = strip.child(self.hero_card(index));
                    }
                    body = body.child(strip).child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(TEXT))
                            .pt(px(18.0))
                            .pb(px(4.0))
                            .child("All apps"),
                    );
                }
                for index in rows {
                    body = body.child(self.row_card(index));
                }
            }
            Tab::Installs | Tab::Updates => {
                if rows.is_empty() {
                    body = body.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED))
                            .child(match self.tab {
                                Tab::Updates => "Everything this machine runs is current.",
                                _ => "Nothing is installed yet.",
                            }),
                    );
                }
                for index in rows {
                    body = body.child(self.row_card(index));
                }
            }
            Tab::Settings => {
                body = body
                    .child(self.setting("Store", self.source.name.clone().unwrap_or_else(|| "none reached".into())))
                    .child(self.setting(
                        "Index",
                        if self.source.verified { "signature verified".into() } else { "unverified".into() },
                    ))
                    .child(self.setting("Machine", format!("{} ({})", self.account.name, self.account.note)))
                    .child(self.setting("Listings", format!("{}", self.source.total)))
                    .child(self.setting("Installed", format!("{}", self.rows.iter().filter(|row| row.state != "available").count())))
                    .child(
                        div()
                            .mt(px(12.0))
                            .w(px(150.0))
                            .py(px(7.0))
                            .rounded(px(8.0))
                            .bg(rgb(PANEL_RAISED))
                            .flex()
                            .justify_center()
                            .text_size(px(11.0))
                            .text_color(rgb(TEXT))
                            .child("Refresh catalog"),
                    );
            }
        }
        let _ = width;
        body.into_any_element()
    }

    fn setting(&self, label: &str, value: String) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .py(px(6.0))
            .w(px(460.0))
            .child(div().text_size(px(12.0)).text_color(rgb(MUTED)).child(label.to_string()))
            .child(div().text_size(px(12.0)).text_color(rgb(TEXT)).child(value))
            .into_any_element()
    }

    fn footer(&self) -> impl IntoElement {
        let (_, left) = self.column();
        div()
            .h(px(FOOTER_H))
            .flex()
            .items_center()
            .justify_between()
            .pl(px(left))
            .pr(px(left))
            .bg(rgb(0x080a0e))
            .text_size(px(10.0))
            .text_color(rgb(MUTED))
            .child(self.message.clone())
            .child("1-4 tabs · enter get/update/open · esc quit")
    }
}

impl Render for StoreView {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .font_family(FONT_FAMILY)
            .child(self.header())
            .child(self.tab_strip())
            .child(div().flex_1().child(self.body()))
            .child(self.footer())
    }
}

/// A name for a row when nobody gave one: the id's last segment, title cased,
/// because `dev.fungos.dfm-report` is a path and "Dfm Report" is a name.
fn title_of(id: &str) -> String {
    let name = id.rsplit('.').next().unwrap_or(id);
    name.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The host's own sentence about a failed call.
fn detail(failure: &catalog::Failure) -> String {
    if failure.detail.trim().is_empty() {
        format!("{:?}", failure.code).to_lowercase()
    } else {
        failure.detail.clone()
    }
}

fn present_frame(
    context: &WgpuContext,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), String> {
    let texture = context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("store frame"),
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
    surface::present(&texture).map_err(|error| format!("present: {error}"))
}

fn render() -> Result<(), String> {
    // The host chooses the extent and the scale; the guest must not assume
    // either, or the frame comes back resampled.
    let (width, height) = surface::size();
    if width == 0 || height == 0 {
        return Err("canvas surface has zero extent".to_string());
    }

    let instance = wgpu_wasi_webgpu::instance();
    let adapter = wit_bindgen::block_on(instance.request_adapter(&Default::default()))
        .map_err(|error| format!("request adapter: {error}"))?;
    let (device, queue) = wit_bindgen::block_on(adapter.request_device(&Default::default()))
        .map_err(|error| format!("request device: {error}"))?;
    let (context, format) = WgpuContext::from_offscreen(instance, adapter, device, queue)
        .map_err(|error| format!("offscreen context: {error}"))?;

    let factory = context.clone();
    let text = Arc::new(CosmicTextSystem::new_without_system_fonts(FONT_FAMILY));
    text.add_fonts(vec![Cow::Borrowed(FONT)])
        .map_err(|error| format!("load store font: {error}"))?;
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(()), move || {
        Ok(Some(Box::new(WgpuHeadlessRenderer::with_context(
            factory.clone(),
            format,
        )?)))
    });

    let (rows, source) = catalog_rows()?;
    let account = account();
    let scale = surface::scale() as f32;
    let window = cx
        .open_window_scaled(size(px(width as f32), px(height as f32)), scale, {
            let rows = rows.clone();
            let source = source.clone();
            let account = account.clone();
            move |_, cx| {
                cx.new(|_| StoreView::new(rows, source, account, width as f32, height as f32))
            }
        })
        .map_err(|error| format!("open headless window: {error}"))?;
    let handle: AnyWindowHandle = window.into();

    // One context, one window, as many frames as the host asks for. A capture
    // host draws once and stops; a live presenter keeps the window, so a redraw
    // costs a draw rather than a device, a font stack and a window.
    let mut dirty = true;
    let mut extent = (width, height);
    loop {
        let now = surface::size();
        if now != extent {
            extent = now;
            window
                .update(&mut cx, |_, window, _| {
                    window.resize(size(px(now.0 as f32), px(now.1 as f32)));
                })
                .map_err(|error| format!("resize: {error}"))?;
            dirty = true;
        }

        if dirty {
            cx.update_window(handle, |_, window, app| {
                let _ = window.draw(app);
            })
            .map_err(|error| format!("draw: {error}"))?;
            let image = cx
                .capture_screenshot(handle)
                .map_err(|error| format!("capture frame: {error}"))?;
            present_frame(&context, image.width(), image.height(), image.as_raw())?;
            dirty = false;
        }

        if !input::interactive() {
            break;
        }

        match input::poll(16) {
            Some(input::Event::KeyInput(key)) => {
                // A release is not an action; the host sends both.
                if !key.pressed {
                    continue;
                }
                if key.key == "escape" {
                    return Ok(());
                }
                let changed = window
                    .update(&mut cx, |view, _, cx| {
                        let changed = view.handle_key(&key.key);
                        if changed {
                            cx.notify();
                        }
                        changed
                    })
                    .map_err(|error| format!("key: {error}"))?;
                dirty |= changed;
            }
            Some(input::Event::Moved(moved)) => {
                let changed = window
                    .update(&mut cx, |view, _, cx| {
                        let target = view.target_at(moved.x, moved.y);
                        let index = match target {
                            Some(Target::Row(index)) | Some(Target::Action(index)) => Some(index),
                            _ => view.hovered,
                        };
                        if index == view.hovered {
                            return false;
                        }
                        view.hovered = index;
                        cx.notify();
                        true
                    })
                    .map_err(|error| format!("move: {error}"))?;
                dirty |= changed;
            }
            Some(input::Event::Button(button)) => {
                if button.pressed && matches!(button.button, input::PointerButton::Left) {
                    let changed = window
                        .update(&mut cx, |view, _, cx| {
                            let target = view.target_at(button.x, button.y);
                            let changed = target.is_some_and(|target| view.activate(target));
                            if changed {
                                cx.notify();
                            }
                            changed
                        })
                        .map_err(|error| format!("click: {error}"))?;
                    dirty |= changed;
                }
            }
            // A wheel is not wired yet: the lists fit the window, and a scroll
            // that moved nothing would be a promise the view cannot keep.
            Some(input::Event::Scrolled(_)) | None => {}
        }
    }
    Ok(())
}
