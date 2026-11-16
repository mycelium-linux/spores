//! The brief: one notification from what the other Spores already recorded.
//!
//! Rules this follows, in the same spirit as the device Spores:
//! - a missing or stale reading is `unknown`, never zero;
//! - every section says where it came from and how old it is;
//! - nothing is computed that the source did not say. The brief composes, it
//!   does not average, estimate, or invent.

use std::cell::RefCell;

use serde_json::{json, Value};
use soil::notify::publisher::{self, Notification};
use soil::notify::types::{Action, Priority};
use soil::records::records::{self, Entry, Key};
use soil::time::time;

wit_bindgen::generate!({ path: "wit", world: "jarvis-brief", generate_all });

const SOURCE_SPORE: &str = "dev.fungos.jarvis-brief";
const SCHEMA: &str = "jarvis.brief.v1";
const PAGE: u32 = 200;

thread_local! {
    static SNAPSHOT: RefCell<Value> = RefCell::new(Value::Null);
}

struct Component;
export!(Component);

#[derive(Debug, Clone)]
struct Args {
    command: String,
    wearer: String,
    now: Option<u64>,
    /// Compose and record, but do not post to the tray.
    dry_run: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "once".to_string(),
            wearer: "owner".to_string(),
            now: None,
            dry_run: false,
        }
    }
}

impl Args {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let mut args = Self::default();
        let mut index = 0;
        if let Some(first) = raw.first() {
            if !first.starts_with("--") {
                args.command = first.clone();
                index = 1;
            }
        }
        while index < raw.len() {
            let flag = raw[index].as_str();
            let mut value = || {
                index += 1;
                raw.get(index)
                    .cloned()
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag {
                "--wearer" => args.wearer = value()?,
                "--now" => {
                    args.now = Some(
                        value()?
                            .parse()
                            .map_err(|_| format!("--now: {:?} is not a number", raw[index]))?,
                    )
                }
                "--dry-run" => args.dry_run = true,
                other => return Err(format!("unknown flag {other:?}")),
            }
            index += 1;
        }
        match args.command.as_str() {
            "once" => Ok(args),
            other => Err(format!("unknown command {other:?}; known: once")),
        }
    }
}

impl Guest for Component {
    fn run(raw: Vec<String>) -> Result<(), String> {
        let result = run_inner(&raw);
        if let Err(error) = &result {
            SNAPSHOT.with(|slot| *slot.borrow_mut() = json!({"ok": false, "error": error}));
        }
        result
    }

    fn snapshot() -> Result<Vec<u8>, String> {
        SNAPSHOT.with(|slot| {
            serde_json::to_vec(&*slot.borrow()).map_err(|error| format!("snapshot JSON: {error}"))
        })
    }
}

fn run_inner(raw: &[String]) -> Result<(), String> {
    let args = Args::parse(raw)?;
    let now = match args.now {
        Some(now) => now,
        None => time::monotonic_ms().map_err(time_err)?,
    };

    let previous = get(&brief_namespace(&args.wearer), &args.wearer)?;
    let last_brief_at_ms = previous
        .as_ref()
        .and_then(|body| body["generated_at_ms"].as_u64());

    let vitals = vitals_section(&args.wearer)?;
    let inbox = inbox_section(&args.wearer, last_brief_at_ms)?;
    let weather = weather_section()?;

    let mut lines = Vec::new();
    if let Some(line) = vitals["line"].as_str() {
        lines.push(line.to_string());
    }
    if let Some(line) = inbox["line"].as_str() {
        lines.push(line.to_string());
    }
    if let Some(line) = weather["line"].as_str() {
        lines.push(line.to_string());
    }
    if lines.is_empty() {
        lines.push("Nothing recorded yet.".to_string());
    }

    let brief = json!({
        "schema": SCHEMA,
        "source_spore": SOURCE_SPORE,
        "wearer": args.wearer,
        "generated_at_ms": now,
        "since_ms": last_brief_at_ms,
        "vitals": vitals,
        "inbox": inbox,
        "weather": weather,
        "summary": lines,
    });
    upsert(&brief_namespace(&args.wearer), &args.wearer, SCHEMA, &brief)?;

    let mut published = false;
    if !args.dry_run {
        let notification = Notification {
            title: "Brief".to_string(),
            body: lines.join("\n"),
            category: "brief".to_string(),
            thread: Some("brief".to_string()),
            group_key: Some("brief".to_string()),
            expires_at_ms: None,
            priority_hint: Some(Priority::Passive),
            sensitive: false,
            actions: vec![Action {
                id: "open-brief".to_string(),
                label: "Open".to_string(),
                destructive: false,
            }],
            context: None,
        };
        publisher::publish(&notification).map_err(notify_err)?;
        published = true;
    }

    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({
            "ok": true,
            "command": "once",
            "wearer": args.wearer,
            "published": published,
            "brief": brief,
        });
    });
    Ok(())
}

/// Vitals come from the view the vitals Spore wrote. A view that is not fresh
/// is reported as unknown: a stale heart rate is not a heart rate.
fn vitals_section(wearer: &str) -> Result<Value, String> {
    let Some(view) = get(&watch_core::namespace("vitals.view", wearer), wearer)? else {
        return Ok(json!({
            "state": "unknown",
            "why": "no vitals view recorded",
            // Said out loud rather than omitted: silence would read as "fine".
            "line": "Vitals: unknown (no vitals view recorded)",
        }));
    };
    let freshness = view["freshness"].as_str().unwrap_or("unknown");
    let mut cards = Vec::new();
    if let Some(items) = view["cards"].as_array() {
        for card in items.iter().take(4) {
            let value = &card["value"];
            if value.is_null() {
                continue;
            }
            let label = card["label"].as_str().unwrap_or_default();
            let unit = card["unit"].as_str().unwrap_or_default();
            let source = card["source_device"].as_str().unwrap_or_default();
            cards.push(format!("{label} {value}{unit} ({source})"));
        }
    }

    let line = if cards.is_empty() {
        format!("Vitals: {freshness}, nothing to report")
    } else {
        format!("Vitals ({freshness}): {}", cards.join(", "))
    };
    Ok(json!({
        "state": if freshness == "fresh" { "fresh" } else { freshness },
        "wearer": view["wearer"],
        "line": line,
        "cards": view["cards"],
    }))
}

/// Messages since the previous brief, counted from the inbox decisions the
/// triage Spore wrote.
fn inbox_section(wearer: &str, since_ms: Option<u64>) -> Result<Value, String> {
    let decisions = scan_all(&watch_core::namespace("inbox", wearer))?;
    let mut held = 0;
    let mut published = 0;
    let mut updated = 0;
    for decision in &decisions {
        let at = decision["posted_at_ms"].as_u64().unwrap_or(0);
        if since_ms.is_some_and(|since| at <= since) {
            continue;
        }
        match decision["action"].as_str() {
            Some("hold") => held += 1,
            Some("publish") => published += 1,
            Some("update") => updated += 1,
            _ => {}
        }
    }
    let total = held + published + updated;
    let line = if total == 0 {
        "Inbox: nothing new".to_string()
    } else {
        format!("Inbox: {published} new, {updated} follow-ups, {held} held")
    };
    Ok(json!({
        "state": "ok",
        "since_ms": since_ms,
        "held": held,
        "published": published,
        "updated": updated,
        "line": line,
    }))
}

/// Weather is global, not per wearer: it is where the wearer is. The newest
/// record wins, and an absent record is unknown rather than clear skies.
fn weather_section() -> Result<Value, String> {
    let records = scan_all("weather.current")?;
    let newest = records
        .iter()
        .max_by_key(|body| body["observed_at_device"].as_u64().unwrap_or(0));
    let Some(record) = newest else {
        return Ok(json!({"state": "unknown", "why": "no weather record"}));
    };
    let location = record["location"].as_str().unwrap_or_default();
    let temp = record["temp_c"].as_f64();
    let min = record["min_c"].as_f64();
    let max = record["max_c"].as_f64();
    let icon = record["icon"].as_str().unwrap_or("").replace('-', " ");
    let line = match (temp, min, max) {
        (Some(temp), Some(min), Some(max)) => {
            format!("{location}: {temp:.1}°C {icon}, {min:.1}..{max:.1}°C")
        }
        _ => format!("{location}: recorded without a temperature"),
    };
    Ok(json!({
        "state": "ok",
        "location": location,
        "temp_c": temp,
        "min_c": min,
        "max_c": max,
        "line": line,
    }))
}

fn brief_namespace(wearer: &str) -> String {
    watch_core::namespace("brief", wearer)
}

fn get(collection: &str, id: &str) -> Result<Option<Value>, String> {
    let key = Key {
        collection: collection.to_string(),
        id: id.to_string(),
    };
    match records::get(&key) {
        Ok(entry) if !entry.deleted => Ok(serde_json::from_slice(&entry.body).ok()),
        Ok(_) => Ok(None),
        Err(records::Error::NotFound) => Ok(None),
        Err(error) => Err(format!("get {collection}/{id}: {error:?}")),
    }
}

fn scan_all(collection: &str) -> Result<Vec<Value>, String> {
    let mut bodies = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let page = records::scan(collection, cursor.as_deref(), PAGE)
            .map_err(|error| format!("scan {collection}: {error:?}"))?;
        for entry in page.entries {
            if entry.deleted {
                continue;
            }
            if let Ok(body) = serde_json::from_slice::<Value>(&entry.body) {
                bodies.push(body);
            }
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(bodies),
        }
    }
    Err(format!("{collection} scan exceeded 20 pages"))
}

fn upsert(collection: &str, id: &str, schema: &str, body: &Value) -> Result<(), String> {
    let entry = Entry {
        key: Key {
            collection: collection.to_string(),
            id: id.to_string(),
        },
        schema: schema.to_string(),
        revision: 0,
        updated_at_ms: 0,
        origin: String::new(),
        deleted: false,
        body: serde_json::to_vec(body).map_err(|error| format!("record JSON: {error}"))?,
    };
    records::upsert(&entry).map_err(|error| format!("{collection}/{id}: {error:?}"))?;
    Ok(())
}

fn notify_err(error: publisher::Error) -> String {
    format!("notify: {error:?}")
}

fn time_err(error: time::Error) -> String {
    format!("time: {error:?}")
}
