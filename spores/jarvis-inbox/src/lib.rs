//! Inbox triage: raw messages in, curated notifications and decisions out.
//!
//! The Spore is deterministic on purpose. A message is classified by rules the
//! wearer can read (VIP senders, breakthrough words, questions, sensitive
//! tokens, thread windows) and every decision is written down with the rule
//! that produced it. Nothing here drafts or sends a reply: outbound goes
//! through `fungos:actions` with an approval, which is a separate capability.
//!
//! Attention policy stays with the tray: this Spore reads `soil:notify/focus`
//! to know whether an item may surface now, but holding, quiet hours and rollup
//! are the host's, so two clients cannot disagree about them.

use std::cell::RefCell;

use serde_json::{json, Value};
use soil::notify::focus;
use soil::notify::publisher::{self, Notification};
use soil::notify::types::{Action, Priority};
use soil::records::records::{self, Entry, Key};
use soil::time::time;

wit_bindgen::generate!({ path: "wit", world: "jarvis-inbox", generate_all });

const SOURCE_SPORE: &str = "dev.fungos.jarvis-inbox";
const RAW_SCHEMA: &str = "inbox.message.v1";
const DECISION_SCHEMA: &str = "inbox.decision.v1";
const NOTIFY_CATEGORY: &str = "message";
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
    /// Senders whose messages are urgent and may break focus.
    vip: Vec<String>,
    /// Words that make a message urgent and may break focus.
    breakthrough: Vec<String>,
    /// Words that mark a body sensitive (OTP, codes, passwords).
    sensitive: Vec<String>,
    /// Messages in one thread inside this window collapse into one item.
    window_ms: u64,
    now: Option<u64>,
    explain: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "once".to_string(),
            wearer: "owner".to_string(),
            vip: Vec::new(),
            breakthrough: Vec::new(),
            sensitive: vec![
                "code".to_string(),
                "otp".to_string(),
                "password".to_string(),
                "verify".to_string(),
            ],
            window_ms: 120_000,
            now: None,
            explain: false,
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
                "--vip" => args.vip = list(&value()?),
                "--breakthrough" => args.breakthrough = list(&value()?),
                "--sensitive" => args.sensitive = list(&value()?),
                "--window-ms" => {
                    args.window_ms = value()?
                        .parse()
                        .map_err(|_| format!("--window-ms: {:?} is not a number", raw[index]))?
                }
                "--now" => {
                    args.now = Some(
                        value()?
                            .parse()
                            .map_err(|_| format!("--now: {:?} is not a number", raw[index]))?,
                    )
                }
                "--explain" => args.explain = true,
                other => return Err(format!("unknown flag {other:?}")),
            }
            index += 1;
        }
        match args.command.as_str() {
            "seed" | "once" => Ok(args),
            other => Err(format!("unknown command {other:?}; known: seed, once")),
        }
    }
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
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
    match args.command.as_str() {
        "seed" => seed(&args, now),
        "once" => once(&args, now),
        other => Err(format!("unknown command {other:?}")),
    }
}

/// A deterministic corpus that covers every rule: VIP, question, sensitive,
/// duplicate thread, and a plain passive message.
fn seed(args: &Args, now: u64) -> Result<(), String> {
    let corpus = [
        ("m1", "sms", "Mom", "mom", "Call me when you can?", 4_000),
        (
            "m2",
            "sms",
            "+15550100",
            "bank-verify",
            "Your verification code is 482913",
            3_000,
        ),
        ("m3", "sms", "Sam", "sam", "lunch at 12?", 2_000),
        ("m4", "sms", "Mom", "mom", "also bring the charger", 1_000),
        (
            "m5",
            "sms",
            "Statements",
            "bank-statements",
            "Your monthly statement is ready",
            500,
        ),
    ];
    for (id, app, sender, thread, body, ago) in corpus {
        let posted_at_ms = now.saturating_sub(ago);
        let record = json!({
            "id": id,
            "app": app,
            "sender": sender,
            "thread": thread,
            "title": sender,
            "body": body,
            "posted_at_ms": posted_at_ms,
        });
        upsert(&raw_namespace(&args.wearer), id, RAW_SCHEMA, &record)?;
    }
    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({
            "ok": true,
            "command": "seed",
            "wearer": args.wearer,
            "messages": 5,
        });
    });
    Ok(())
}

fn once(args: &Args, now: u64) -> Result<(), String> {
    let decisions = scan_all(&decision_namespace(&args.wearer))?;
    let processed: Vec<&str> = decisions
        .iter()
        .filter_map(|body| body["message_id"].as_str())
        .collect();

    let focus = focus::get().map_err(focus_err)?;
    let mode = focus.as_ref().map(|profile| profile.mode.clone());

    let mut raw = scan_all(&raw_namespace(&args.wearer))?;
    raw.retain(|body| {
        body["id"]
            .as_str()
            .is_some_and(|id| !processed.contains(&id))
    });
    raw.sort_by_key(|body| body["posted_at_ms"].as_u64().unwrap_or(0));

    // thread -> (notify id, last posted at, count) so a second message in one
    // thread updates the item instead of posting another.
    let mut threads = thread_map(&decisions);
    let mut counts = Counts::default();
    let mut held = Vec::new();

    for message in &raw {
        let id = message["id"].as_str().unwrap_or_default();
        let sender = message["sender"].as_str().unwrap_or_default();
        let thread = message["thread"].as_str().unwrap_or_default();
        let app = message["app"].as_str().unwrap_or("sms");
        let body = message["body"].as_str().unwrap_or_default();
        let posted_at_ms = message["posted_at_ms"].as_u64().unwrap_or(now);

        let vip = matches(sender, &args.vip);
        let breakthrough = matches(body, &args.breakthrough);
        let sensitive = matches(body, &args.sensitive);

        if !allowed(focus.as_ref(), app, thread, breakthrough.is_some()) {
            held.push(id.to_string());
            counts.held += 1;
            let decision = json!({
                "message_id": id,
                "wearer": args.wearer,
                "action": "hold",
                "rule": "focus",
                "why": format!("focus mode {}", mode.as_deref().unwrap_or("on")),
                "thread": thread,
                "posted_at_ms": posted_at_ms,
            });
            decide(args, id, &decision)?;
            explain(args, "hold", id, "focus");
            continue;
        }

        let (priority, rule) = if vip.is_some() {
            (Priority::Urgent, "vip")
        } else if breakthrough.is_some() {
            (Priority::Urgent, "breakthrough")
        } else if body.contains('?') {
            (Priority::Active, "question")
        } else {
            (Priority::Passive, "normal")
        };

        let notification = notification(sender, body, thread, priority, sensitive.is_some());

        if let Some(entry) = threads.get(thread) {
            if now.saturating_sub(entry.last_at_ms) <= args.window_ms {
                let count = entry.count + 1;
                let notify_id = entry.notify_id.clone();
                let mut updated = notification.clone();
                updated.title = format!("{sender} ({count} new)");
                publisher::update(&notify_id, &updated).map_err(notify_err)?;
                threads.insert(
                    thread.to_string(),
                    ThreadEntry {
                        notify_id: notify_id.clone(),
                        last_at_ms: posted_at_ms,
                        count,
                    },
                );
                counts.updated += 1;
                let decision = json!({
                    "message_id": id,
                    "wearer": args.wearer,
                    "action": "update",
                    "rule": rule,
                    "why": format!("{count} messages in thread {thread}"),
                    "thread": thread,
                    "notify_id": notify_id,
                    "count": count,
                    "posted_at_ms": posted_at_ms,
                });
                decide(args, id, &decision)?;
                explain(args, "update", id, rule);
                continue;
            }
        }

        let notify_id = publisher::publish(&notification).map_err(notify_err)?;
        threads.insert(
            thread.to_string(),
            ThreadEntry {
                notify_id: notify_id.clone(),
                last_at_ms: posted_at_ms,
                count: 1,
            },
        );
        counts.published += 1;
        let decision = json!({
            "message_id": id,
            "wearer": args.wearer,
            "action": "publish",
            "rule": rule,
            "priority": priority_name(priority),
            "sensitive": sensitive.is_some(),
            "thread": thread,
            "notify_id": notify_id,
            "count": 1,
            "posted_at_ms": posted_at_ms,
        });
        decide(args, id, &decision)?;
        explain(args, "publish", id, rule);
    }

    if !held.is_empty() && focus.as_ref().is_some_and(|profile| profile.rollup_held) {
        let digest = Notification {
            title: format!("Held during {}", mode.as_deref().unwrap_or("focus")),
            body: format!("{} message(s)", held.len()),
            category: "digest".to_string(),
            thread: None,
            group_key: Some("held".to_string()),
            expires_at_ms: None,
            priority_hint: Some(Priority::Passive),
            sensitive: false,
            actions: vec![Action {
                id: "open-inbox".to_string(),
                label: "Open".to_string(),
                destructive: false,
            }],
            context: None,
        };
        publisher::publish(&digest).map_err(notify_err)?;
        counts.digest += 1;
        explain(args, "digest", &format!("{} held", held.len()), "rollup");
    }

    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({
            "ok": true,
            "command": "once",
            "wearer": args.wearer,
            "focus": mode,
            "processed": raw.len(),
            "published": counts.published,
            "updated": counts.updated,
            "held": counts.held,
            "digest": counts.digest,
        });
    });
    Ok(())
}

#[derive(Default)]
struct Counts {
    published: u64,
    updated: u64,
    held: u64,
    digest: u64,
}

struct ThreadEntry {
    notify_id: String,
    last_at_ms: u64,
    count: u64,
}

fn thread_map(decisions: &[Value]) -> std::collections::BTreeMap<String, ThreadEntry> {
    let mut map = std::collections::BTreeMap::new();
    for decision in decisions {
        let (Some(thread), Some(notify_id)) =
            (decision["thread"].as_str(), decision["notify_id"].as_str())
        else {
            continue;
        };
        if thread.is_empty() {
            continue;
        }
        let last_at_ms = decision["posted_at_ms"].as_u64().unwrap_or(0);
        let count = decision["count"].as_u64().unwrap_or(1);
        map.insert(
            thread.to_string(),
            ThreadEntry {
                notify_id: notify_id.to_string(),
                last_at_ms,
                count,
            },
        );
    }
    map
}

/// May this message surface now? Focus is host policy; breakthrough words and
/// VIP senders are the only things a message itself can use to override it.
fn allowed(profile: Option<&focus::Profile>, app: &str, thread: &str, breakthrough: bool) -> bool {
    let Some(profile) = profile else {
        return true;
    };
    if profile.mode == "off" || breakthrough {
        return true;
    }
    contains(&profile.allow_apps, app)
        || contains(&profile.allow_categories, NOTIFY_CATEGORY)
        || contains(&profile.allow_threads, thread)
}

fn contains(list: &[String], value: &str) -> bool {
    list.iter().any(|entry| entry.eq_ignore_ascii_case(value))
}

/// The first token that appears in the text, case-insensitively.
fn matches(text: &str, tokens: &[String]) -> Option<String> {
    let text = text.to_ascii_lowercase();
    tokens
        .iter()
        .find(|token| text.contains(token.as_str()))
        .cloned()
}

fn notification(
    sender: &str,
    body: &str,
    thread: &str,
    priority: Priority,
    sensitive: bool,
) -> Notification {
    Notification {
        title: sender.to_string(),
        body: body.to_string(),
        category: NOTIFY_CATEGORY.to_string(),
        thread: Some(thread.to_string()),
        group_key: Some(thread.to_string()),
        expires_at_ms: None,
        priority_hint: Some(priority),
        sensitive,
        actions: vec![
            Action {
                id: "reply-later".to_string(),
                label: "Later".to_string(),
                destructive: false,
            },
            Action {
                id: "mute-thread".to_string(),
                label: "Mute thread".to_string(),
                destructive: false,
            },
        ],
        context: None,
    }
}

fn priority_name(priority: Priority) -> &'static str {
    match priority {
        Priority::Urgent => "urgent",
        Priority::Active => "active",
        Priority::Passive => "passive",
    }
}

fn explain(args: &Args, action: &str, id: &str, rule: &str) {
    if args.explain {
        println!("inbox: {action} {id} by {rule}");
    }
}

fn decide(args: &Args, message_id: &str, decision: &Value) -> Result<(), String> {
    let mut body = decision.clone();
    if let Some(object) = body.as_object_mut() {
        object.insert("source_spore".to_string(), json!(SOURCE_SPORE));
    }
    upsert(
        &decision_namespace(&args.wearer),
        message_id,
        DECISION_SCHEMA,
        &body,
    )
}

fn raw_namespace(wearer: &str) -> String {
    watch_core::namespace("inbox.raw", wearer)
}

fn decision_namespace(wearer: &str) -> String {
    watch_core::namespace("inbox", wearer)
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

fn focus_err(error: focus::Error) -> String {
    format!("focus: {error:?}")
}

fn time_err(error: time::Error) -> String {
    format!("time: {error:?}")
}
