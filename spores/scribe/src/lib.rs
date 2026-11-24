//! Scribe: durable capture bundles from Unibus into VFS + records.
//!
//! The Spore decides the bundle layout and the `fungos.transcript/capture@1`
//! shape. The host owns bytes in VFS, record durability, indexing, and the
//! Unibus transport. Nothing here uses a socket, filesystem path, secret, or
//! wall clock.

use std::cell::RefCell;

use serde_json::{json, Value};
use soil::index::index;
use soil::inference::inference;
use soil::records::records::{self, Entry, Key};
use soil::streams::duplex;
use soil::time::time;
use soil::unibus::bus;
use soil::vfs::vfs;
use soil::vfs_mut::vfs_mut;

wit_bindgen::generate!({ path: "wit", world: "scribe", generate_all });

const SOURCE_SPORE: &str = "dev.fungos.scribe";

thread_local! {
    static SNAPSHOT: RefCell<Value> = const { RefCell::new(Value::Null) };
}

struct Component;
export!(Component);

#[derive(Debug, Clone)]
struct Args {
    command: String,
    hold_ms: u64,
    once: bool,
    event: Option<String>,
    model: String,
    enrich: bool,
    path: Option<String>,
    templates: String,
    templates_json: Option<String>,
    checkpoint: String,
    threshold: f64,
    margin: f64,
    deep_model: String,
    deep_after_chars: usize,
    force: bool,
    limit: usize,
    gap_ms: u64,
    retry_failed: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "watch".into(),
            hold_ms: 5_000,
            once: false,
            event: None,
            model: "scribe/summarize".to_string(),
            enrich: true,
            path: None,
            templates: "voiceprints/templates.json".to_string(),
            templates_json: None,
            checkpoint: "sha256:394365a16176e8a002f0151b4ee8b1cc1f73d031cdfaa337d0bfb4eaf86676fb"
                .to_string(),
            threshold: 0.80,
            margin: 0.08,
            deep_model: "beam-free".to_string(),
            deep_after_chars: 80_000,
            force: false,
            limit: 0,
            gap_ms: 10_000,
            retry_failed: false,
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
                "--hold-ms" => {
                    args.hold_ms = value()?
                        .parse()
                        .map_err(|_| "--hold-ms must be a number".to_string())?
                }
                "--once" => args.once = true,
                "--event" => args.event = Some(value()?),
                "--model" => args.model = value()?,
                "--no-enrich" => args.enrich = false,
                "--path" => args.path = Some(value()?),
                "--templates" => args.templates = value()?,
                "--templates-json" => args.templates_json = Some(value()?),
                "--checkpoint" => args.checkpoint = value()?,
                "--threshold" => {
                    args.threshold = value()?
                        .parse()
                        .map_err(|_| "--threshold must be a number".to_string())?
                }
                "--margin" => {
                    args.margin = value()?
                        .parse()
                        .map_err(|_| "--margin must be a number".to_string())?
                }
                "--deep-model" => args.deep_model = value()?,
                "--deep-after-chars" => {
                    args.deep_after_chars = value()?
                        .parse()
                        .map_err(|_| "--deep-after-chars must be a number".to_string())?
                }
                "--force" => args.force = true,
                "--limit" => {
                    args.limit = value()?
                        .parse()
                        .map_err(|_| "--limit must be a number".to_string())?
                }
                "--gap-ms" => {
                    args.gap_ms = value()?
                        .parse()
                        .map_err(|_| "--gap-ms must be a number".to_string())?
                }
                "--retry-failed" => args.retry_failed = true,
                "--help" | "-h" => {
                    return Err(
                        "scribe commands: watch [--hold-ms N] [--once] | receive [--once] | enrich [--hold-ms N] [--once] | voiceprint --templates-json JSON | voiceprint --path PATH | voicefix --path PATH | names --path PATH | learn | ingest --event JSON [--model ALIAS] [--no-enrich]"
                            .into(),
                    )
                }
                other => return Err(format!("unknown flag {other}")),
            }
            index += 1;
        }
        Ok(args)
    }
}

impl Guest for Component {
    fn run(raw: Vec<String>) -> Result<(), String> {
        let result = run_inner(&raw);
        if let Err(error) = &result {
            SNAPSHOT.with(|slot| {
                *slot.borrow_mut() = json!({"ok": false, "error": error});
            });
        }
        result
    }

    fn snapshot() -> Result<Vec<u8>, String> {
        SNAPSHOT.with(|slot| {
            serde_json::to_vec(&*slot.borrow()).map_err(|error| format!("snapshot: {error}"))
        })
    }
}

fn run_inner(raw: &[String]) -> Result<(), String> {
    let args = Args::parse(raw)?;
    match args.command.as_str() {
        "ingest" => {
            let raw_event = args
                .event
                .as_deref()
                .ok_or_else(|| "ingest needs --event JSON".to_string())?;
            let body: Value =
                serde_json::from_str(raw_event).map_err(|error| format!("event JSON: {error}"))?;
            let stored = process_event(
                &body,
                &args.model,
                args.enrich,
                true,
                &args.deep_model,
                args.deep_after_chars,
            )?;
            finish(json!({"ok": true, "command": "ingest", "capture": stored}));
            Ok(())
        }
        "watch" => watch(args),
        "receive" => receive(args),
        "enrich" => enrich_watch(args),
        "backfill" => backfill(args),
        "voiceprint" => voiceprint(args),
        "voicefix" => voicefix(args),
        "restyle" => restyle(args),
        "clean" => clean(args),
        "names" => names(args),
        "learn" => learn_command(args),
        other => Err(format!("unknown command {other}")),
    }
}

const MAX_STREAM_PATH: usize = 1024;
const MAX_STREAM_FILE: u64 = 2 * 60 * 60 * 16_000 * 2 + 4096;
const STREAM_CHUNK: usize = 64 * 1024;

/// Accept one host-owned WSS bundle, then run the normal Scribe pipeline.
/// The wire format is Scribe's capture transport, not a second product:
///
/// ```text
/// "OMI1"
/// repeat: u16 path-len, path, u64 data-len, data
/// u16 0
/// ```
fn receive(args: Args) -> Result<(), String> {
    let mut stored = 0_u64;
    loop {
        let Some(session) = duplex::accept("wss").map_err(|error| format!("accept: {error:?}"))?
        else {
            time::tick(50).map_err(|error| format!("tick: {error:?}"))?;
            continue;
        };
        match receive_one(session, &args) {
            Ok(event) => {
                stored += 1;
                eprintln!(
                    "scribe: received {} ({} segment(s))",
                    event["capture_id"].as_str().unwrap_or("capture"),
                    event["segments"].as_u64().unwrap_or(0)
                );
            }
            Err(error) => eprintln!("scribe: receive failed: {error}"),
        }
        if args.once {
            break;
        }
    }
    finish(json!({"ok": true, "command": "receive", "stored": stored}));
    Ok(())
}

fn receive_one(mut session: duplex::Session, args: &Args) -> Result<Value, String> {
    let mut reader = StreamReader::new(&mut session);
    let mut magic = [0u8; 4];
    if !reader.read_exact(&mut magic)? || &magic != b"OMI1" {
        return Err("bad capture stream magic".into());
    }
    let mut event_bytes: Option<Vec<u8>> = None;
    loop {
        let Some(path_len) = reader.read_u16()? else {
            break;
        };
        let path_len = path_len as usize;
        if path_len == 0 {
            break;
        }
        if path_len > MAX_STREAM_PATH {
            return Err(format!("bad path length {path_len}"));
        }
        let mut path_raw = vec![0u8; path_len];
        if !reader.read_exact(&mut path_raw)? {
            return Err("truncated path".into());
        }
        let path = String::from_utf8(path_raw).map_err(|_| "path is not utf-8")?;
        if !stream_path(&path) {
            return Err(format!("unsafe path {path:?}"));
        }
        let len = reader
            .read_u64()?
            .ok_or_else(|| format!("missing data length for {path}"))?;
        if len > MAX_STREAM_FILE {
            return Err(format!("file {path} is too large: {len}"));
        }
        let writer = vfs_mut::begin_write(&path, stream_media_type(&path), Some(len), None, None)
            .map_err(|error| format!("begin-write {path}: {error:?}"))?;
        let mut remaining = len;
        let mut kept = Vec::new();
        while remaining > 0 {
            let want = remaining.min(STREAM_CHUNK as u64) as usize;
            let chunk = reader
                .read_some(want)?
                .ok_or_else(|| format!("truncated data for {path}"))?;
            if path.ends_with("/event.json") && kept.len() < 4 * 1024 * 1024 {
                let left = (4 * 1024 * 1024 - kept.len()).min(chunk.len());
                kept.extend_from_slice(&chunk[..left]);
            }
            writer
                .write(&chunk)
                .map_err(|error| format!("write {path}: {error:?}"))?;
            remaining -= chunk.len() as u64;
        }
        writer
            .finish()
            .map_err(|error| format!("finish {path}: {error:?}"))?;
        if path.ends_with("/event.json") {
            event_bytes = Some(kept);
        }
    }
    let raw = event_bytes.ok_or_else(|| "capture stream had no event.json".to_string())?;
    let body: Value =
        serde_json::from_slice(&raw).map_err(|error| format!("event JSON: {error}"))?;
    let body = body.get("event").cloned().unwrap_or(body);
    let checkpoint = body
        .get("checkpoint")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stored = process_event(
        &body,
        &args.model,
        false,
        !checkpoint,
        &args.deep_model,
        args.deep_after_chars,
    )?;
    session
        .write(b"OK\n")
        .map_err(|error| format!("ack: {error:?}"))?;
    session
        .close()
        .map_err(|error| format!("close: {error:?}"))?;
    Ok(stored)
}

fn stream_path(path: &str) -> bool {
    path.starts_with("captures/")
        && !path.contains("..")
        && !path.contains('\\')
        && !path.ends_with('/')
        && path.len() <= MAX_STREAM_PATH
}

fn stream_media_type(path: &str) -> &'static str {
    if path.ends_with(".wav") {
        "audio/wav"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".md") {
        "text/markdown; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

struct StreamReader<'a> {
    session: &'a mut duplex::Session,
    buffer: Vec<u8>,
    offset: usize,
}

impl<'a> StreamReader<'a> {
    fn new(session: &'a mut duplex::Session) -> Self {
        Self {
            session,
            buffer: Vec::new(),
            offset: 0,
        }
    }

    fn fill(&mut self) -> Result<bool, String> {
        if self.offset < self.buffer.len() {
            return Ok(true);
        }
        self.buffer.clear();
        self.offset = 0;
        match self
            .session
            .read(STREAM_CHUNK as u32)
            .map_err(|error| format!("read: {error:?}"))?
        {
            Some(bytes) if !bytes.is_empty() => {
                self.buffer = bytes;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn read_some(&mut self, max: usize) -> Result<Option<Vec<u8>>, String> {
        if !self.fill()? {
            return Ok(None);
        }
        let take = (self.buffer.len() - self.offset).min(max);
        let out = self.buffer[self.offset..self.offset + take].to_vec();
        self.offset += take;
        Ok(Some(out))
    }

    fn read_exact(&mut self, out: &mut [u8]) -> Result<bool, String> {
        let mut at = 0;
        while at < out.len() {
            if !self.fill()? {
                return Ok(false);
            }
            let take = (self.buffer.len() - self.offset).min(out.len() - at);
            out[at..at + take].copy_from_slice(&self.buffer[self.offset..self.offset + take]);
            self.offset += take;
            at += take;
        }
        Ok(true)
    }

    fn read_u16(&mut self) -> Result<Option<u16>, String> {
        let mut bytes = [0u8; 2];
        if !self.read_exact(&mut bytes)? {
            return Ok(None);
        }
        Ok(Some(u16::from_be_bytes(bytes)))
    }

    fn read_u64(&mut self) -> Result<Option<u64>, String> {
        let mut bytes = [0u8; 8];
        if !self.read_exact(&mut bytes)? {
            return Ok(None);
        }
        Ok(Some(u64::from_be_bytes(bytes)))
    }
}

/// Consume Scribe's own durable handoff. This is deliberately a separate
/// process from `receive`: capture acknowledgement stays fast, and the LLM can
/// fail or lag without holding the transport open.
fn enrich_watch(args: Args) -> Result<(), String> {
    let started = now_ms()?;
    let mut cursor: Option<String> = None;
    let mut seen = 0_u64;
    let mut enriched = 0_u64;
    let mut skipped = 0_u64;
    let mut failed = 0_u64;
    loop {
        if let Err(error) = learn_manual(&args) {
            eprintln!("scribe: manual voice learning: {error}");
        }
        if let Err(error) = voicefix_queue(&args) {
            eprintln!("scribe: line voice fixes: {error}");
        }
        if let Err(error) = restyle_queue(&args) {
            eprintln!("scribe: summary restyle: {error}");
        }
        if let Err(error) = clean_queue(&args) {
            eprintln!("scribe: transcript cleanup: {error}");
        }
        if let Err(error) = merge_queue(&args) {
            eprintln!("scribe: merge queue: {error}");
        }
        let page = bus::poll(&["scribe.capture.stored.v1".into()], cursor.as_deref(), 32)
            .map_err(|error| format!("unibus poll: {error:?}"))?;
        for frame in page.frames {
            seen += 1;
            let body: Value = match serde_json::from_slice(&frame.body) {
                Ok(body) => body,
                Err(error) => {
                    failed += 1;
                    eprintln!("scribe: enrich event JSON: {error}");
                    continue;
                }
            };
            if body.get("enrichment").and_then(Value::as_str) != Some("pending") {
                skipped += 1;
                continue;
            }
            let Some(bundle) = body.get("bundle").and_then(Value::as_str) else {
                failed += 1;
                eprintln!("scribe: pending capture event has no bundle");
                continue;
            };
            match load_bundle_event(&body, bundle) {
                Ok(mut event) => {
                    // The stored event is already durable; don't republish it
                    // and create a second enrichment turn.
                    event["checkpoint"] = Value::Bool(false);
                    match process_event(
                        &event,
                        &args.model,
                        true,
                        false,
                        &args.deep_model,
                        args.deep_after_chars,
                    ) {
                        Ok(_) => {
                            enriched += 1;
                            if let Err(error) = voiceprint_capture(&args, bundle) {
                                eprintln!("scribe: voiceprint {bundle}: {error}");
                            }
                        }
                        Err(error) => {
                            failed += 1;
                            eprintln!("scribe: enrich {bundle}: {error}");
                        }
                    }
                }
                Err(error) => {
                    failed += 1;
                    eprintln!("scribe: load {bundle}: {error}");
                }
            }
        }
        cursor = page.next;
        if args.once || now_ms()?.saturating_sub(started) >= args.hold_ms {
            break;
        }
        let _ = time::tick(250);
    }
    finish(json!({
        "ok": true,
        "command": "enrich",
        "seen": seen,
        "enriched": enriched,
        "skipped": skipped,
        "failed": failed,
    }));
    Ok(())
}

fn backfill(args: Args) -> Result<(), String> {
    let roots = capture_roots()?;
    let state_path = "scribe/backfill-state.json";
    let mut state = read_json(state_path).unwrap_or_else(|_| json!({}));
    if !state.is_object() {
        state = json!({});
    }
    let mut seen = 0_u64;
    let mut work = 0_u64;
    let mut enriched = 0_u64;
    let mut skipped = 0_u64;
    let mut failed = 0_u64;
    for root in roots {
        seen += 1;
        let summary = read_text(&format!("{root}/summary.md")).unwrap_or_default();
        if !summary.trim().is_empty() {
            skipped += 1;
            continue;
        }
        let attempts = state
            .get(&root)
            .and_then(|value| value.get("attempts"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if attempts >= 3 && !args.retry_failed {
            skipped += 1;
            continue;
        }
        if args.limit > 0 && work >= args.limit as u64 {
            break;
        }
        work += 1;
        let id = root.rsplit('/').next().unwrap_or("capture").to_string();
        let source = root.split('/').nth(1).unwrap_or("omi").to_string();
        let meta = read_json(&format!("{root}/capture.json")).unwrap_or_else(|_| json!({}));
        let stored = json!({
            "capture_id": id,
            "source": source,
            "bundle": root,
            "started_at_ms": meta.get("started_at_ms").cloned().unwrap_or(Value::Null),
            "ended_at_ms": meta.get("ended_at_ms").cloned().unwrap_or(Value::Null),
        });
        match load_bundle_event(&stored, &root).and_then(|mut event| {
            event["checkpoint"] = Value::Bool(false);
            process_event(
                &event,
                &args.model,
                true,
                false,
                &args.deep_model,
                args.deep_after_chars,
            )
        }) {
            Ok(_) => {
                if let Some(fields) = state.as_object_mut() {
                    fields.remove(&root);
                }
                enriched += 1;
                eprintln!("scribe: backfilled {root}");
            }
            Err(error) => {
                failed += 1;
                if let Some(fields) = state.as_object_mut() {
                    fields.insert(root.clone(), json!({
                        "attempts": attempts + 1,
                        "last_error": error,
                    }));
                }
                eprintln!("scribe: backfill {root}: {error}");
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&state).map_err(|error| error.to_string())?;
    vfs_mut::write(state_path, &bytes, "application/json", None, None)
        .map_err(|error| format!("write backfill state: {error:?}"))?;
    finish(json!({
        "ok": true,
        "command": "backfill",
        "seen": seen,
        "enriched": enriched,
        "skipped": skipped,
        "failed": failed,
    }));
    Ok(())
}


fn capture_roots() -> Result<Vec<String>, String> {
    let mut roots = Vec::new();
    for source in list_dirs("captures")? {
        for date in list_dirs(&source)? {
            for capture in list_dirs(&date)? {
                roots.push(capture);
            }
        }
    }
    roots.sort();
    Ok(roots)
}

fn list_dirs(parent: &str) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    let mut offset = 0u64;
    loop {
        let page = vfs::ls(parent, offset, 256).map_err(|error| format!("vfs ls {parent}: {error:?}"))?;
        for entry in page.entries {
            if entry.kind == vfs::Kind::Directory {
                paths.push(entry.path);
            }
        }
        match page.next {
            Some(next) => offset = next,
            None => break,
        }
    }
    Ok(paths)
}

fn load_bundle_event(stored: &Value, bundle: &str) -> Result<Value, String> {
    let transcript = read_json(&format!("{bundle}/transcript.json"))?;
    let meta = read_json(&format!("{bundle}/capture.json")).unwrap_or_else(|_| json!({}));
    let summary = read_text(&format!("{bundle}/summary.omi.md"))
        .or_else(|_| read_text(&format!("{bundle}/summary.md")))
        .unwrap_or_default();
    let actions = read_json(&format!("{bundle}/actions.omi.json"))
        .or_else(|_| read_json(&format!("{bundle}/actions.json")))
        .unwrap_or_else(|_| json!([]));
    let questions = read_json(&format!("{bundle}/questions.json")).unwrap_or_else(|_| json!([]));
    let calendar = read_json(&format!("{bundle}/calendar.json")).unwrap_or_else(|_| json!([]));
    Ok(json!({
        "capture_id": stored.get("capture_id").cloned().unwrap_or(Value::Null),
        "source": stored.get("source").cloned().unwrap_or_else(|| json!("omi")),
        "date": meta.get("date").cloned().unwrap_or(Value::Null),
        "started_at_ms": stored.get("started_at_ms").cloned().or_else(|| meta.get("started_at_ms").cloned()).unwrap_or(Value::Null),
        "ended_at_ms": stored.get("ended_at_ms").cloned().or_else(|| meta.get("ended_at_ms").cloned()).unwrap_or(Value::Null),
        "title": meta.get("title").cloned().unwrap_or(Value::Null),
        "emoji": meta.get("emoji").cloned().unwrap_or(Value::Null),
        "summary_md": summary,
        "actions": actions,
        "questions": questions,
        "calendar": calendar,
        "transcript": transcript,
        "checkpoint": false,
    }))
}

fn read_text(path: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut offset = 0u64;
    loop {
        let chunk = vfs::read(path, offset, 1024 * 1024).map_err(|error| format!("{error:?}"))?;
        let len = chunk.data.len();
        bytes.extend(chunk.data);
        if chunk.eof || len == 0 {
            break;
        }
        offset += len as u64;
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn read_json(path: &str) -> Result<Value, String> {
    serde_json::from_str(&read_text(path)?).map_err(|error| error.to_string())
}

fn learn_command(args: Args) -> Result<(), String> {
    let result = learn_manual(&args)?;
    finish(json!({"ok": true, "command": "learn", "result": result}));
    Ok(())
}

/// Turn manual speaker tags into reusable voice templates. The queue is
/// written by Scribe UI; this worker owns the audio/model call so the HTTP
/// path stays fast and a missing embedding backend can retry later.
fn learn_manual(args: &Args) -> Result<Value, String> {
    let queue_path = "voiceprints/learn.json";
    let mut queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    if !queue.is_array() {
        queue = json!([]);
    }
    let Some(items) = queue.as_array().cloned() else {
        return Ok(json!({"learned": 0, "pending": 0}));
    };
    if items.is_empty() {
        return Ok(json!({"learned": 0, "pending": 0}));
    }
    let mut templates = read_json(&args.templates).unwrap_or_else(|_| {
        json!({
            "model": "speechbrain/spkrec-ecapa-voxceleb@sha256:394365a16176e8a002f0151b4ee8b1cc1f73d031cdfaa337d0bfb4eaf86676fb",
            "pipeline": "ecapa-mono16k-v1",
            "templates": []
        })
    });
    let model = templates
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("speechbrain/spkrec-ecapa-voxceleb@sha256:394365a16176e8a002f0151b4ee8b1cc1f73d031cdfaa337d0bfb4eaf86676fb")
        .to_string();
    let pipeline = templates
        .get("pipeline")
        .and_then(Value::as_str)
        .unwrap_or("ecapa-mono16k-v1")
        .to_string();
    if !templates.get("templates").is_some_and(Value::is_array) {
        templates["templates"] = json!([]);
    }
    let mut learned = 0usize;
    let mut pending = Vec::new();
    for item in items {
        let path = item.get("path").and_then(Value::as_str).unwrap_or("");
        let speaker = item.get("speaker").and_then(Value::as_str).unwrap_or("");
        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
        let requested_person_id = item
            .get("person_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(name);
        // A display name can already belong to an enrolled person under a
        // different stable id (for example Avery is `ajmwagar`). Reuse that
        // identity instead of creating a second template for the same voice.
        let person_id = templates
            .get("templates")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter().find(|template| {
                    template
                        .get("display_name")
                        .and_then(Value::as_str)
                        .is_some_and(|display| display.eq_ignore_ascii_case(name))
                })
            })
            .and_then(|template| template.get("person_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| requested_person_id.to_string());
        if path.is_empty() || speaker.is_empty() || name.is_empty() {
            continue;
        }
        let force = item.get("force").and_then(Value::as_bool).unwrap_or(false);
        // A manual tag may be replayed after a worker restart. If that person
        // is already enrolled, acknowledge the queue item without doing the
        // audio round trip again unless this is an explicit dataset update.
        if !force
            && templates
                .get("templates")
                .and_then(Value::as_array)
                .is_some_and(|list| {
                    list.iter().any(|template| {
                        template.get("person_id").and_then(Value::as_str)
                            == Some(person_id.as_str())
                            && template
                                .get("embedding")
                                .and_then(Value::as_array)
                                .is_some_and(|embedding| !embedding.is_empty())
                    })
                })
        {
            continue;
        }
        let transcript = match read_json(&format!("{path}/transcript.json")) {
            Ok(value) => value,
            Err(_) => {
                pending.push(item.clone());
                continue;
            }
        };
        let requested_window = item
            .get("start")
            .and_then(Value::as_f64)
            .zip(item.get("end").and_then(Value::as_f64))
            .filter(|(start, end)| end > start && *start >= 0.0);
        let Some((start, end)) = requested_window.or_else(|| speaker_window(&transcript, speaker))
        else {
            pending.push(item.clone());
            continue;
        };
        let pcm = match wav_pcm_segment(&format!("{path}/audio.wav"), start, end) {
            Ok(pcm) if pcm.len() >= 32_000 => pcm,
            _ => {
                pending.push(item.clone());
                continue;
            }
        };
        let request = inference::VoiceEmbeddingRequest {
            model: "fpl/spkrec-ecapa-voxceleb".to_string(),
            checkpoint: args.checkpoint.clone(),
            sample_rate: 16_000,
            dimensions: Some(192),
            pcm16: pcm,
        };
        let response = match inference::voice_embed(&request) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("scribe: learn {path} {speaker}: {error:?}");
                pending.push(item.clone());
                continue;
            }
        };
        let embedding: Vec<f64> = response
            .embedding
            .iter()
            .map(|value| *value as f64)
            .collect();
        if normalize_embedding(&embedding).is_err() {
            pending.push(item.clone());
            continue;
        }
        let update = json!({
            "person_id": person_id,
            "display_name": name,
            "model": model,
            "pipeline": pipeline,
            "embedding": embedding,
        });
        if let Some(list) = templates.get_mut("templates").and_then(Value::as_array_mut) {
            if let Some(existing) = list.iter_mut().find(|template| {
                template.get("person_id").and_then(Value::as_str) == Some(person_id.as_str())
            }) {
                *existing = update;
            } else {
                list.push(update);
            }
        }
        learned += 1;
    }
    if learned > 0 {
        let bytes = serde_json::to_vec_pretty(&templates).map_err(|error| error.to_string())?;
        vfs_mut::write(&args.templates, &bytes, "application/json", None, None)
            .map_err(|error| format!("write templates: {error:?}"))?;
    }
    let remaining = serde_json::to_vec_pretty(&pending).map_err(|error| error.to_string())?;
    vfs_mut::write(queue_path, &remaining, "application/json", None, None)
        .map_err(|error| format!("write learning queue: {error:?}"))?;
    Ok(json!({"learned": learned, "pending": pending.len()}))
}

fn speaker_key(item: &Value) -> String {
    if item.get("is_user").and_then(Value::as_bool) == Some(true) {
        return "You".to_string();
    }
    item.get("speaker")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            item.get("speaker")
                .and_then(Value::as_i64)
                .map(|value| format!("SPEAKER_{value:02}"))
        })
        .unwrap_or_else(|| "Speaker".to_string())
}

/// Return the best embeddable window for every speaker.
///
/// Word-level transcripts have no utterance spans, so a single longest word is
/// far too short for ECAPA. Group contiguous words from the same speaker into
/// turns first, then cap the selected turn to a bounded ECAPA probe window.
fn speaker_windows(transcript: &Value) -> std::collections::BTreeMap<String, (f64, f64)> {
    let mut windows = std::collections::BTreeMap::new();
    let mut add = |key: &str, start: f64, end: f64| {
        if end <= start {
            return;
        }
        let capped_end = (start + (end - start).min(24.0)).max(start + 1.0);
        let entry = windows
            .entry(key.to_string())
            .or_insert((start, capped_end));
        if capped_end - start > entry.1 - entry.0 {
            *entry = (start, capped_end);
        }
    };
    if let Some(segments) = transcript.get("segments").and_then(Value::as_array) {
        if !segments.is_empty() {
            for segment in segments {
                let start = segment.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let end = segment.get("end").and_then(Value::as_f64).unwrap_or(start);
                add(&speaker_key(segment), start, end);
            }
            return windows;
        }
    }
    let Some(words) = transcript.get("words").and_then(Value::as_array) else {
        return windows;
    };
    let mut current: Option<(String, f64, f64)> = None;
    let flush = |turn: Option<(String, f64, f64)>,
                 windows: &mut std::collections::BTreeMap<String, (f64, f64)>| {
        if let Some((key, start, end)) = turn {
            let capped_end = (start + (end - start).min(24.0)).max(start + 1.0);
            let entry = windows.entry(key).or_insert((start, capped_end));
            if capped_end - start > entry.1 - entry.0 {
                *entry = (start, capped_end);
            }
        }
    };
    for word in words {
        let key = speaker_key(word);
        let start = word.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        let end = word.get("end").and_then(Value::as_f64).unwrap_or(start);
        if end <= start {
            continue;
        }
        match current.as_mut() {
            Some((running_key, _, running_end))
                if *running_key == key && start - *running_end <= 2.0 =>
            {
                *running_end = end;
            }
            _ => {
                flush(current.take(), &mut windows);
                current = Some((key, start, end));
            }
        }
    }
    flush(current.take(), &mut windows);
    windows
}

fn speaker_window(transcript: &Value, wanted: &str) -> Option<(f64, f64)> {
    speaker_windows(transcript).get(wanted).copied()
}

fn voiceprint(args: Args) -> Result<(), String> {
    if let Some(raw) = args.templates_json.as_deref() {
        let value: Value =
            serde_json::from_str(raw).map_err(|error| format!("templates JSON: {error}"))?;
        let bytes = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
        vfs_mut::write(&args.templates, &bytes, "application/json", None, None)
            .map_err(|error| format!("write templates: {error:?}"))?;
        finish(json!({"ok": true, "command": "voiceprint", "templates": args.templates}));
        return Ok(());
    }
    let path = args
        .path
        .as_deref()
        .ok_or("voiceprint needs --path or --templates-json")?;
    let result = voiceprint_capture(&args, path)?;
    finish(json!({"ok": true, "command": "voiceprint", "capture": path, "result": result}));
    Ok(())
}

fn voiceprint_capture(args: &Args, path: &str) -> Result<Value, String> {
    let templates = match read_json(&args.templates) {
        Ok(value) => value,
        Err(_) => return Ok(json!({"status": "no templates"})),
    };
    let items = templates
        .get("templates")
        .and_then(Value::as_array)
        .ok_or("templates JSON needs a templates array")?;
    if items.is_empty() {
        return Ok(json!({"status": "no templates"}));
    }
    let transcript = read_json(&format!("{path}/transcript.json"))?;
    let by_speaker = speaker_windows(&transcript);
    if by_speaker.is_empty() {
        return Err("transcript has no speaker timestamps".into());
    }
    let audio = format!("{path}/audio.wav");
    let mut speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
    if !speakers.is_object() {
        speakers = json!({});
    }
    let mut recognized = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for (speaker, (start, end)) in by_speaker {
        if !args.force
            && speakers
                .get(&speaker)
                .and_then(|entry| entry.get("source"))
                .and_then(Value::as_str)
                == Some("manual")
        {
            continue;
        }
        let pcm = match wav_pcm_segment(&audio, start, end) {
            Ok(pcm) if pcm.len() >= 32_000 => pcm,
            Ok(pcm) => {
                eprintln!(
                    "scribe: voiceprint {speaker} audio too short: {} bytes",
                    pcm.len()
                );
                continue;
            }
            Err(error) => {
                errors.push(format!("{speaker} audio: {error}"));
                continue;
            }
        };
        eprintln!(
            "scribe: voiceprint {speaker} {start:.1}-{end:.1}s pcm={}",
            pcm.len()
        );
        let request = inference::VoiceEmbeddingRequest {
            model: "fpl/spkrec-ecapa-voxceleb".to_string(),
            checkpoint: args.checkpoint.clone(),
            sample_rate: 16_000,
            dimensions: Some(192),
            pcm16: pcm,
        };
        let response = match inference::voice_embed(&request) {
            Ok(response) => response,
            Err(error) => {
                errors.push(format!("{speaker} embedding: {error:?}"));
                continue;
            }
        };
        let response_embedding: Vec<f64> = response
            .embedding
            .iter()
            .map(|value| *value as f64)
            .collect();
        let probe = match normalize_embedding(&response_embedding) {
            Ok(probe) => probe,
            Err(error) => {
                errors.push(format!("{speaker} normalize: {error}"));
                continue;
            }
        };
        let mut ranked: Vec<(f64, &Value)> = items
            .iter()
            .filter_map(|item| {
                let embedding = item.get("embedding")?.as_array()?;
                let values = embedding
                    .iter()
                    .map(Value::as_f64)
                    .collect::<Option<Vec<f64>>>()?;
                Some((cosine(&probe, &normalize_embedding(&values).ok()?), item))
            })
            .collect();
        ranked.sort_by(|left, right| right.0.total_cmp(&left.0));
        let Some((best, template)) = ranked.first() else {
            continue;
        };
        let runner_up = ranked.get(1).map(|entry| entry.0).unwrap_or(-1.0);
        if *best < args.threshold || *best - runner_up < args.margin {
            speakers[&speaker] = json!({
                "source": "jarvis",
                "person_id": Value::Null,
                "name": Value::Null,
                "score": best,
                "best_score": best,
                "model": response.model,
                "pipeline": "ecapa-mono16k-v1",
            });
            continue;
        }
        let person_id = template
            .get("person_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let name = template
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(person_id);
        speakers[&speaker] = json!({
            "source": "jarvis",
            "person_id": person_id,
            "name": name,
            "score": best,
            "model": response.model,
            "pipeline": "ecapa-mono16k-v1",
        });
        recognized += 1;
    }
    let bytes = serde_json::to_vec_pretty(&speakers).map_err(|error| error.to_string())?;
    vfs_mut::write(
        &format!("{path}/speakers.json"),
        &bytes,
        "application/json",
        None,
        None,
    )
    .map_err(|error| format!("write speakers: {error:?}"))?;
    let _ = apply_speaker_names(path, &speakers);
    Ok(json!({"recognized": recognized, "speakers": speakers, "errors": errors}))
}

fn turn_windows(transcript: &Value) -> Vec<(String, f64, f64)> {
    let mut turns = Vec::new();
    if let Some(segments) = transcript.get("segments").and_then(Value::as_array) {
        if !segments.is_empty() {
            for segment in segments {
                let start = segment.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let end = segment.get("end").and_then(Value::as_f64).unwrap_or(start);
                if end > start {
                    turns.push((speaker_key(segment), start, end));
                }
            }
            return turns;
        }
    }
    let Some(words) = transcript.get("words").and_then(Value::as_array) else {
        return turns;
    };
    let mut current: Option<(String, f64, f64)> = None;
    for word in words {
        let key = speaker_key(word);
        let start = word.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        let end = word.get("end").and_then(Value::as_f64).unwrap_or(start);
        if end <= start {
            continue;
        }
        match current.as_mut() {
            Some((running_key, _, running_end))
                if *running_key == key && start - *running_end <= 1.8 =>
            {
                *running_end = end;
            }
            _ => {
                if let Some(turn) = current.take() {
                    turns.push(turn);
                }
                current = Some((key, start, end));
            }
        }
    }
    if let Some(turn) = current {
        turns.push(turn);
    }
    turns
}

fn voicefix(args: Args) -> Result<(), String> {
    let path = args.path.as_deref().ok_or("voicefix needs --path")?;
    let result = voicefix_capture(&args, path)?;
    finish(json!({"ok": true, "command": "voicefix", "capture": path, "result": result}));
    Ok(())
}

fn voicefix_queue(args: &Args) -> Result<(), String> {
    let queue_path = "voiceprints/voicefix.json";
    let queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    let Some(items) = queue.as_array() else {
        return Ok(());
    };
    if items.is_empty() {
        return Ok(());
    }
    let mut pending = Vec::new();
    for item in items {
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        match voicefix_capture(args, path) {
            Ok(_) => {}
            Err(error) => {
                eprintln!("scribe: voicefix {path}: {error}");
                pending.push(item.clone());
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&json!(pending)).map_err(|error| error.to_string())?;
    vfs_mut::write(queue_path, &bytes, "application/json", None, None)
        .map_err(|error| format!("write voicefix queue: {error:?}"))?;
    Ok(())
}

fn voicefix_capture(args: &Args, path: &str) -> Result<Value, String> {
    let templates = read_json(&args.templates).map_err(|error| format!("templates: {error}"))?;
    let items = templates
        .get("templates")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or("no voice templates")?;
    let transcript = read_json(&format!("{path}/transcript.json"))?;
    let turns = turn_windows(&transcript);
    let overrides_path = format!("{path}/speaker_overrides.json");
    let existing = read_json(&overrides_path).unwrap_or_else(|_| json!([]));
    let mut manual: Vec<(f64, f64)> = Vec::new();
    let mut next: Vec<Value> = Vec::new();
    if let Some(entries) = existing.as_array() {
        for entry in entries {
            if entry.get("source").and_then(Value::as_str) == Some("manual") {
                let start = entry.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let end = entry.get("end").and_then(Value::as_f64).unwrap_or(start);
                manual.push((start, end));
                next.push(entry.clone());
            }
        }
    }
    let audio = format!("{path}/audio.wav");
    let mut matched = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for (speaker, turn_start, turn_end) in turns {
        if turn_end - turn_start < 0.4 {
            skipped += 1;
            continue;
        }
        if manual
            .iter()
            .any(|(start, end)| turn_start < *end && turn_end > *start)
        {
            skipped += 1;
            continue;
        }
        let start = turn_start.max(0.0);
        let end = (turn_start + (turn_end - turn_start).min(24.0)).max(start + 1.0);
        let pcm = match wav_pcm_segment(&audio, start, end) {
            Ok(pcm) if pcm.len() >= 32_000 => pcm,
            _ => {
                skipped += 1;
                continue;
            }
        };
        let request = inference::VoiceEmbeddingRequest {
            model: "fpl/spkrec-ecapa-voxceleb".to_string(),
            checkpoint: args.checkpoint.clone(),
            sample_rate: 16_000,
            dimensions: Some(192),
            pcm16: pcm,
        };
        let response = match inference::voice_embed(&request) {
            Ok(response) => response,
            Err(error) => {
                errors.push(format!("{start:.1}-{end:.1}: {error:?}"));
                continue;
            }
        };
        let probe: Vec<f64> = response
            .embedding
            .iter()
            .map(|value| *value as f64)
            .collect();
        let Ok(probe) = normalize_embedding(&probe) else {
            skipped += 1;
            continue;
        };
        let mut ranked: Vec<(f64, &Value)> = items
            .iter()
            .filter_map(|template| {
                let embedding = template.get("embedding")?.as_array()?;
                let values = embedding
                    .iter()
                    .map(Value::as_f64)
                    .collect::<Option<Vec<f64>>>()?;
                Some((
                    cosine(&probe, &normalize_embedding(&values).ok()?),
                    template,
                ))
            })
            .collect();
        ranked.sort_by(|left, right| right.0.total_cmp(&left.0));
        let Some((best, template)) = ranked.first() else {
            continue;
        };
        let runner_up = ranked.get(1).map(|entry| entry.0).unwrap_or(-1.0);
        if *best < args.threshold || *best - runner_up < args.margin {
            skipped += 1;
            continue;
        }
        let person_id = template
            .get("person_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let name = template
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(person_id);
        next.push(json!({
            "start": start,
            "end": end,
            "speaker": speaker,
            "name": name,
            "person_id": person_id,
            "score": best,
            "model": response.model,
            "pipeline": "ecapa-mono16k-v1",
            "source": "voice",
        }));
        matched += 1;
    }
    next.sort_by(|left, right| {
        left.get("start")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .total_cmp(&right.get("start").and_then(Value::as_f64).unwrap_or(0.0))
    });
    let bytes = serde_json::to_vec_pretty(&json!(next)).map_err(|error| error.to_string())?;
    vfs_mut::write(&overrides_path, &bytes, "application/json", None, None)
        .map_err(|error| format!("write overrides: {error:?}"))?;
    Ok(json!({"turns": matched, "skipped": skipped, "errors": errors}))
}

fn restyle(args: Args) -> Result<(), String> {
    let path = args.path.as_deref().ok_or("restyle needs --path")?;
    let result = restyle_capture(&args, path)?;
    finish(json!({"ok": true, "command": "restyle", "capture": path, "result": result}));
    Ok(())
}

fn restyle_queue(args: &Args) -> Result<(), String> {
    let queue_path = "voiceprints/restyle.json";
    let queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    let Some(items) = queue.as_array() else {
        return Ok(());
    };
    if items.is_empty() {
        return Ok(());
    }
    let mut pending = Vec::new();
    for item in items {
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        match restyle_capture(args, path) {
            Ok(_) => {}
            Err(error) => {
                eprintln!("scribe: restyle {path}: {error}");
                pending.push(item.clone());
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&json!(pending)).map_err(|error| error.to_string())?;
    vfs_mut::write(queue_path, &bytes, "application/json", None, None)
        .map_err(|error| format!("write restyle queue: {error:?}"))?;
    Ok(())
}

fn restyle_capture(args: &Args, path: &str) -> Result<Value, String> {
    let meta = read_json(&format!("{path}/capture.json")).unwrap_or_else(|_| json!({}));
    let speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
    let mut source = read_text(&format!("{path}/summary.md")).unwrap_or_default();
    if source.trim().is_empty() {
        source = transcript_text(
            &read_json(&format!("{path}/transcript.json")).unwrap_or_else(|_| json!({})),
        );
    }
    if let Some(entries) = speakers.as_object() {
        for (label, entry) in entries {
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                source = source.replace(label, name);
            }
        }
    }
    let transcript = json!({"text": source});
    let capture_id = meta
        .get("capture_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string());
    let event = json!({
        "capture_id": capture_id,
        "source": meta.get("source").and_then(Value::as_str).unwrap_or("omi"),
        "date": meta.get("date").and_then(Value::as_str),
        "started_at_ms": meta.get("started_at_ms").cloned(),
        "ended_at_ms": meta.get("ended_at_ms").cloned(),
        "title": meta.get("title").cloned(),
        "summary_md": "",
        "actions": [],
        "transcript": transcript,
    });
    let event = parse_event(&event)?;
    let enriched = enrich_capture(
        &event,
        &args.model,
        &args.deep_model,
        args.deep_after_chars,
        true,
    )?;
    let mut design = enriched.summary;
    if let Some(entries) = speakers.as_object() {
        for (label, entry) in entries {
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                design = design.replace(label, name);
            }
        }
    }
    let summary = write_file(
        &format!("{path}/summary.design.md"),
        design.as_bytes(),
        "text/markdown",
    )?;
    let index_path = format!("{path}/summary.json");
    let mut index = read_json(&index_path).unwrap_or_else(|_| json!({}));
    index["design"] = json!({
        "path": summary.path,
        "content_id": summary.content_id,
        "model": enriched.model,
    });
    let bytes = serde_json::to_vec_pretty(&index).map_err(|error| error.to_string())?;
    write_file(&index_path, &bytes, "application/json")?;
    Ok(json!({"summary": summary.path, "model": enriched.model}))
}

fn clean(args: Args) -> Result<(), String> {
    let path = args.path.as_deref().ok_or("clean needs --path")?;
    let result = clean_capture(&args, path)?;
    finish(json!({"ok": true, "command": "clean", "capture": path, "result": result}));
    Ok(())
}

fn clean_queue(args: &Args) -> Result<(), String> {
    let queue_path = "scribe/clean.json";
    let queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    let Some(items) = queue.as_array() else {
        return Ok(());
    };
    if items.is_empty() {
        return Ok(());
    }
    let mut pending = Vec::new();
    for item in items {
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        match clean_capture(args, path) {
            Ok(_) => {}
            Err(error) => {
                eprintln!("scribe: clean {path}: {error}");
                pending.push(item.clone());
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&json!(pending)).map_err(|error| error.to_string())?;
    vfs_mut::write(queue_path, &bytes, "application/json", None, None)
        .map_err(|error| format!("write clean queue: {error:?}"))?;
    Ok(())
}

fn clean_capture(args: &Args, path: &str) -> Result<Value, String> {
    let transcript = read_json(&format!("{path}/transcript.json"))?;
    let speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
    let vocabulary = read_text("scribe/vocabulary.txt").unwrap_or_default();
    let mut turns = Vec::new();
    let mut current: Option<(String, String, f64, f64, Vec<String>)> = None;
    let mut push_turn = |turn: Option<(String, String, f64, f64, Vec<String>)>,
                         turns: &mut Vec<Value>| {
        if let Some((key, name, start, end, words)) = turn {
            if !words.is_empty() {
                turns.push(json!({
                    "id": turns.len(),
                    "start": start,
                    "end": end,
                    "speaker_key": key,
                    "speaker": name,
                    "text": words.join(" "),
                }));
            }
        }
    };
    if let Some(segments) = transcript.get("segments").and_then(Value::as_array) {
        if !segments.is_empty() {
            for segment in segments {
                let key = speaker_key(segment);
                let name = speakers
                    .get(&key)
                    .and_then(|entry| entry.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(&key)
                    .to_string();
                let start = segment.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let end = segment.get("end").and_then(Value::as_f64).unwrap_or(start);
                let text = segment
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if !text.is_empty() {
                    turns.push(json!({
                        "id": turns.len(),
                        "start": start,
                        "end": end,
                        "speaker_key": key,
                        "speaker": name,
                        "text": text,
                    }));
                }
            }
        }
    }
    if turns.is_empty() {
        if let Some(words) = transcript.get("words").and_then(Value::as_array) {
            for word in words {
                let key = speaker_key(word);
                let token = word
                    .get("word")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let start = word.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let end = word.get("end").and_then(Value::as_f64).unwrap_or(start);
                if token.is_empty() || end <= start {
                    continue;
                }
                let name = speakers
                    .get(&key)
                    .and_then(|entry| entry.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(&key)
                    .to_string();
                match current.as_mut() {
                    Some((running_key, _, _, running_end, text))
                        if *running_key == key && start - *running_end <= 1.8 =>
                    {
                        text.push(token.to_string());
                        *running_end = end;
                    }
                    _ => {
                        push_turn(current.take(), &mut turns);
                        current = Some((key, name, start, end, vec![token.to_string()]));
                    }
                }
            }
            push_turn(current.take(), &mut turns);
        }
    }
    if turns.is_empty() {
        return Err("transcript has no readable turns".into());
    }
    let vocab = vocabulary.trim();
    let mut cleaned: Vec<Value> = Vec::new();
    let mut offset = 0usize;
    while offset < turns.len() {
        let mut end = offset;
        let mut chars = 0usize;
        while end < turns.len() && (end == offset || chars < 120_000) {
            chars += turns[end]
                .get("text")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or(0);
            end += 1;
        }
        let batch = &turns[offset..end];
        let input = serde_json::to_string_pretty(&batch).map_err(|error| error.to_string())?;
        let system = format!(
            "You are Scribe's transcript editor. Return ONLY JSON: {{\"turns\":[{{\"id\":number,\"text\":string}}]}}. Fix punctuation, capitalization, spacing, and obvious ASR errors. Remove filler only when it carries no meaning. Preserve every turn id, speaker attribution, numbers, names, technical terms, and meaning. Do not merge, split, or reorder turns. Known vocabulary (preserve exact spelling): {vocab}"
        );
        let request = inference::Request {
            model: args.model.clone(),
            messages: vec![
                inference::Message {
                    role: inference::Role::System,
                    content: Some(system),
                    name: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                    attachments: vec![],
                    reasoning: None,
                },
                inference::Message {
                    role: inference::Role::User,
                    content: Some(input),
                    name: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                    attachments: vec![],
                    reasoning: None,
                },
            ],
            tools: vec![],
            max_output_tokens: Some(32_768),
            temperature: Some(0.1),
            prompt_cache_key: Some(format!("scribe-clean:{}", path)),
        };
        let returned = match inference::complete(&request) {
            Ok(response) => response
                .message
                .content
                .as_deref()
                .and_then(|content| parse_model_json(content).ok()),
            Err(error) => {
                eprintln!("scribe: clean batch inference: {error:?}");
                None
            }
        };
        let returned = returned
            .as_ref()
            .and_then(|value| value.get("turns"))
            .and_then(Value::as_array);
        for turn in batch {
            let id = turn.get("id").and_then(Value::as_u64).unwrap_or(0);
            let text = returned
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("id").and_then(Value::as_u64) == Some(id))
                        .and_then(|item| item.get("text"))
                        .and_then(Value::as_str)
                })
                .unwrap_or_else(|| turn.get("text").and_then(Value::as_str).unwrap_or(""));
            let mut out = turn.clone();
            out["text"] = json!(text);
            cleaned.push(out);
        }
        offset = end;
    }
    let body = json!({
        "model": args.model,
        "vocabulary": vocab,
        "turns": cleaned,
    });
    let bytes = serde_json::to_vec_pretty(&body).map_err(|error| error.to_string())?;
    write_file(
        &format!("{path}/transcript.clean.json"),
        &bytes,
        "application/json",
    )?;
    Ok(json!({"turns": cleaned.len(), "path": format!("{path}/transcript.clean.json")}))
}

fn names(args: Args) -> Result<(), String> {
    let path = args.path.as_deref().ok_or("names needs --path")?;
    let speakers = read_json(&format!("{path}/speakers.json"))?;
    let changed = apply_speaker_names(path, &speakers)?;
    finish(json!({"ok": true, "command": "names", "capture": path, "changed": changed}));
    Ok(())
}

/// Replace raw diarization labels in action text once a speaker has a name.
/// The labels are opaque UI ids, not content; leaving them in action titles or
/// quotes makes the proposal harder to read than the transcript it came from.
fn apply_speaker_names(path: &str, speakers: &Value) -> Result<bool, String> {
    let Some(entries) = speakers.as_object() else {
        return Ok(false);
    };
    let labels: Vec<(String, String)> = entries
        .iter()
        .filter_map(|(label, entry)| {
            let name = entry.get("name").and_then(Value::as_str)?.trim();
            (!name.is_empty()).then(|| (label.clone(), name.to_string()))
        })
        .collect();
    if labels.is_empty() {
        return Ok(false);
    }
    let replace = |text: &str| {
        let mut next = text.to_string();
        for (label, name) in &labels {
            next = next.replace(label, name);
        }
        next
    };
    let mut changed = false;
    let actions_path = format!("{path}/actions.json");
    if let Ok(mut actions) = read_json(&actions_path) {
        if let Some(items) = actions.as_array_mut() {
            for action in items {
                let Some(fields) = action.as_object_mut() else {
                    continue;
                };
                for field in ["title", "quote", "owner", "speaker"] {
                    let Some(value) = fields
                        .get(field)
                        .and_then(Value::as_str)
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    let next = replace(&value);
                    if next != value {
                        fields.insert(field.to_string(), Value::String(next));
                        changed = true;
                    }
                }
            }
        }
        if changed {
            let bytes = serde_json::to_vec_pretty(&actions).map_err(|error| error.to_string())?;
            vfs_mut::write(&actions_path, &bytes, "application/json", None, None)
                .map_err(|error| format!("write actions: {error:?}"))?;
        }
    }
    for file in ["summary.md", "summary.design.md"] {
        let summary_path = format!("{path}/{file}");
        let Ok(text) = read_text(&summary_path) else {
            continue;
        };
        let next = replace(&text);
        if next != text {
            write_file(&summary_path, next.as_bytes(), "text/markdown")?;
            changed = true;
        }
    }
    Ok(changed)
}

fn wav_pcm_segment(path: &str, start: f64, end: f64) -> Result<Vec<u8>, String> {
    let header = read_vfs_bytes(path, 0, 4096)?;
    let data_at = header
        .windows(4)
        .position(|window| window == b"data")
        .map(|at| at + 8)
        .unwrap_or(44) as u64;
    let start_at = data_at + (start.max(0.0) * 32_000.0) as u64;
    let wanted = ((end - start).clamp(1.0, 24.0) * 32_000.0) as u64;
    read_vfs_bytes(path, start_at, wanted)
}

fn read_vfs_bytes(path: &str, offset: u64, length: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(length as usize);
    let mut at = offset;
    while (bytes.len() as u64) < length {
        let want = (length - bytes.len() as u64).min(1024 * 1024) as u32;
        let chunk = vfs::read(path, at, want).map_err(|error| format!("{error:?}"))?;
        let len = chunk.data.len();
        bytes.extend(chunk.data);
        if chunk.eof || len == 0 {
            break;
        }
        at += len as u64;
    }
    Ok(bytes)
}

fn normalize_embedding(values: &[f64]) -> Result<Vec<f64>, String> {
    if values.len() != 192 || values.iter().any(|value| !value.is_finite()) {
        return Err("expected 192 finite embedding values".into());
    }
    let norm = values.iter().map(|value| value * value).sum::<f64>().sqrt();
    if norm < 1e-12 {
        return Err("zero-norm embedding".into());
    }
    Ok(values.iter().map(|value| value / norm).collect())
}

fn cosine(left: &[f64], right: &[f64]) -> f64 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

#[derive(Clone)]
struct MergeSource {
    path: String,
    meta: Value,
    transcript: Value,
    speakers: Value,
    overrides: Value,
    actions: Value,
    questions: Value,
    calendar: Value,
    summary: String,
    audio: Option<WavAudio>,
    started_at_ms: u64,
    ended_at_ms: u64,
}

#[derive(Clone)]
struct WavAudio {
    sample_rate: u32,
    channels: u16,
    bits: u16,
    pcm: Vec<u8>,
}

fn merge_queue(args: &Args) -> Result<(), String> {
    let queue_path = "scribe/merge.json";
    let queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    let Some(items) = queue.as_array().cloned() else {
        return Ok(());
    };
    if items.is_empty() {
        return Ok(());
    }
    let mut pending = Vec::new();
    let mut results = Vec::new();
    for item in items {
        let paths: Vec<String> = item
            .get("paths")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        match merge_captures(&paths, args) {
            Ok(value) => results.push(value),
            Err(error) => {
                eprintln!("scribe: merge: {error}");
                results.push(json!({"ok": false, "paths": paths, "error": error}));
                pending.push(item);
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&json!(pending)).map_err(|error| error.to_string())?;
    vfs_mut::write(queue_path, &bytes, "application/json", None, None)
        .map_err(|error| format!("write merge queue: {error:?}"))?;
    let result_bytes =
        serde_json::to_vec_pretty(&json!(results)).map_err(|error| error.to_string())?;
    vfs_mut::write(
        "scribe/merge-result.json",
        &result_bytes,
        "application/json",
        None,
        None,
    )
    .map_err(|error| format!("write merge result: {error:?}"))?;
    Ok(())
}

fn merge_captures(paths: &[String], args: &Args) -> Result<Value, String> {
    let mut seen = std::collections::BTreeSet::new();
    let paths: Vec<String> = paths
        .iter()
        .cloned()
        .filter(|path| valid_capture_path(path) && seen.insert(path.clone()))
        .collect();
    if paths.len() < 2 {
        return Err("merge needs at least two distinct capture paths".into());
    }
    let mut sources = Vec::new();
    for path in paths.iter() {
        let meta = read_json(&format!("{path}/capture.json")).unwrap_or_else(|_| json!({}));
        if meta.get("merged_into").and_then(Value::as_str).is_some()
            || meta.get("merged_from").is_some()
        {
            return Err(format!("{path} is already merged"));
        }
        let transcript = read_json(&format!("{path}/transcript.json"))?;
        let speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
        let overrides =
            read_json(&format!("{path}/speaker_overrides.json")).unwrap_or_else(|_| json!([]));
        let actions = read_json(&format!("{path}/actions.json")).unwrap_or_else(|_| json!([]));
        let questions = read_json(&format!("{path}/questions.json")).unwrap_or_else(|_| json!([]));
        let calendar = read_json(&format!("{path}/calendar.json")).unwrap_or_else(|_| json!([]));
        let summary = read_text(&format!("{path}/summary.md")).unwrap_or_default();
        let started_at_ms = meta
            .get("started_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let ended_at_ms = meta
            .get("ended_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(started_at_ms);
        let audio = read_wav(&format!("{path}/audio.wav"));
        sources.push(MergeSource {
            path: path.clone(),
            meta,
            transcript,
            speakers,
            overrides,
            actions,
            questions,
            calendar,
            summary,
            audio,
            started_at_ms,
            ended_at_ms,
        });
    }
    sources.sort_by_key(|source| source.started_at_ms);
    let earliest = sources[0].started_at_ms;
    let latest = sources
        .iter()
        .map(|source| source.ended_at_ms)
        .max()
        .unwrap_or(earliest);
    let source_name = sources[0]
        .path
        .split('/')
        .nth(1)
        .unwrap_or("omi")
        .to_string();
    let date = sources[0]
        .path
        .split('/')
        .nth(2)
        .unwrap_or("undated")
        .to_string();
    let target_id = unique_merge_id(&source_name, &date, &sources[0].path);
    let root = format!("captures/{source_name}/{date}/{target_id}");

    let (speaker_map, speakers) = merge_speakers(&sources);
    let mut merged_segments = Vec::new();
    let mut merged_words = Vec::new();
    let mut text_parts = Vec::new();
    let mut merged_actions = Vec::new();
    let mut merged_questions = Vec::new();
    let mut merged_calendar = Vec::new();
    let mut merged_overrides = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let offset_ms = source.started_at_ms.saturating_sub(earliest) as f64;
        let mut transcript = source.transcript.clone();
        remap_transcript_speakers(&mut transcript, &speaker_map[index]);
        offset_transcript(&mut transcript, offset_ms / 1000.0);
        if let Some(text) = transcript.get("text").and_then(Value::as_str) {
            if !text.trim().is_empty() {
                text_parts.push(text.trim().to_string());
            }
        }
        if let Some(segments) = transcript.get("segments").and_then(Value::as_array) {
            merged_segments.extend(segments.iter().cloned());
        }
        if let Some(words) = transcript.get("words").and_then(Value::as_array) {
            merged_words.extend(words.iter().cloned());
        }
        if let Some(items) = source.actions.as_array() {
            for item in items {
                let mut item = item.clone();
                if let Some(fields) = item.as_object_mut() {
                    for key in ["start_ms", "end_ms"] {
                        if let Some(value) = fields.get(key).and_then(Value::as_f64) {
                            fields[key] = json!(value + offset_ms);
                        }
                    }
                    for key in ["speaker", "owner"] {
                        if let Some(value) = fields.get(key).and_then(Value::as_str) {
                            if let Some(mapped) = speaker_map[index].get(value) {
                                fields[key] = json!(mapped);
                            }
                        }
                    }
                    fields["source_capture"] = json!(source.path);
                }
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let quote = item
                    .get("quote")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let duplicate = merged_actions.iter().any(|existing: &Value| {
                    existing
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_lowercase()
                        == title
                        && existing
                            .get("quote")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_lowercase()
                            == quote
                });
                if !duplicate {
                    merged_actions.push(item);
                }
            }
        }
        if let Some(items) = source.questions.as_array() {
            for item in items {
                let mut item = item.clone();
                if let Some(fields) = item.as_object_mut() {
                    if let Some(value) = fields.get("start_ms").and_then(Value::as_f64) {
                        fields["start_ms"] = json!(value + offset_ms);
                    }
                    fields["source_capture"] = json!(source.path);
                }
                let question = item
                    .get("question")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let quote = item
                    .get("quote")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                if !merged_questions.iter().any(|existing: &Value| {
                    existing
                        .get("question")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_lowercase()
                        == question
                        && existing
                            .get("quote")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_lowercase()
                            == quote
                }) {
                    merged_questions.push(item);
                }
            }
        }
        if let Some(items) = source.calendar.as_array() {
            for item in items {
                let mut item = item.clone();
                if let Some(fields) = item.as_object_mut() {
                    if let Some(value) = fields.get("start_ms").and_then(Value::as_f64) {
                        fields["start_ms"] = json!(value + offset_ms);
                    }
                    fields["source_capture"] = json!(source.path);
                }
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let start = item.get("start").and_then(Value::as_str).unwrap_or("");
                if !merged_calendar.iter().any(|existing: &Value| {
                    existing
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_lowercase()
                        == title
                        && existing.get("start").and_then(Value::as_str).unwrap_or("") == start
                }) {
                    merged_calendar.push(item);
                }
            }
        }
        if let Some(items) = source.overrides.as_array() {
            for item in items {
                let mut item = item.clone();
                if let Some(fields) = item.as_object_mut() {
                    for key in ["start", "end"] {
                        if let Some(value) = fields.get(key).and_then(Value::as_f64) {
                            fields[key] = json!(value + offset_ms / 1000.0);
                        }
                    }
                    if let Some(speaker) = fields.get("speaker").and_then(Value::as_str) {
                        if let Some(mapped) = speaker_map[index].get(speaker) {
                            fields["speaker"] = json!(mapped);
                        }
                    }
                }
                merged_overrides.push(item);
            }
        }
    }
    merged_segments.sort_by(|left, right| {
        left.get("start")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .total_cmp(&right.get("start").and_then(Value::as_f64).unwrap_or(0.0))
    });
    merged_words.sort_by(|left, right| {
        left.get("start")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .total_cmp(&right.get("start").and_then(Value::as_f64).unwrap_or(0.0))
    });
    let summary = sources
        .iter()
        .map(|source| source.summary.trim())
        .filter(|summary| !summary.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    let title = sources[0]
        .meta
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Merged capture");
    let emoji = sources[0]
        .meta
        .get("emoji")
        .and_then(Value::as_str)
        .unwrap_or("");
    let event = json!({
        "capture_id": target_id,
        "source": source_name,
        "date": date,
        "started_at_ms": earliest,
        "ended_at_ms": latest,
        "title": title,
        "emoji": emoji,
        "summary_md": summary,
        "actions": merged_actions,
        "questions": merged_questions,
        "calendar": merged_calendar,
        "transcript": {
            "text": text_parts.join(" "),
            "segments": merged_segments,
            "words": merged_words,
        },
        "merged_from": paths,
        "checkpoint": false,
    });
    let merged = process_event(
        &event,
        &args.model,
        true,
        false,
        &args.deep_model,
        args.deep_after_chars,
    )?;
    if let Some(audio) = merge_audio(&sources, earliest) {
        write_file(&format!("{root}/audio.wav"), &audio, "audio/wav")?;
    }

    let merged_paths: Vec<Value> = sources.iter().map(|source| json!(source.path)).collect();
    let mut meta = read_json(&format!("{root}/capture.json")).unwrap_or_else(|_| json!({}));
    if let Some(fields) = meta.as_object_mut() {
        fields["merged_from"] = json!(merged_paths);
        fields["emoji"] = json!(if emoji.trim().is_empty() {
            emoji_for(title, &summary)
        } else {
            emoji
        });
    }
    let meta_bytes = serde_json::to_vec_pretty(&meta).map_err(|error| error.to_string())?;
    write_file(
        &format!("{root}/capture.json"),
        &meta_bytes,
        "application/json",
    )?;
    if !merged_overrides.is_empty() {
        let bytes = serde_json::to_vec_pretty(&json!(merged_overrides))
            .map_err(|error| error.to_string())?;
        write_file(
            &format!("{root}/speaker_overrides.json"),
            &bytes,
            "application/json",
        )?;
    }
    let speakers_bytes = serde_json::to_vec_pretty(&speakers).map_err(|error| error.to_string())?;
    write_file(
        &format!("{root}/speakers.json"),
        &speakers_bytes,
        "application/json",
    )?;

    for source in &sources {
        let mut meta = source.meta.clone();
        if let Some(fields) = meta.as_object_mut() {
            fields["merged_into"] = json!(root);
        }
        let bytes = serde_json::to_vec_pretty(&meta).map_err(|error| error.to_string())?;
        let _ = write_file(
            &format!("{}/capture.json", source.path),
            &bytes,
            "application/json",
        );
    }
    let _ = index::refresh();
    Ok(json!({"ok": true, "bundle": root, "captures": merged, "sources": paths}))
}

fn valid_capture_path(path: &str) -> bool {
    path.starts_with("captures/")
        && !path.contains("..")
        && path.split('/').count() >= 4
        && !path.ends_with('/')
}

fn unique_merge_id(source: &str, date: &str, first: &str) -> String {
    let first = first.rsplit('/').next().unwrap_or("capture");
    let mut index = 0;
    loop {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let id = sanitize(&format!("merged-{first}{suffix}"))
            .unwrap_or_else(|_| format!("merged-{index}"));
        let path = format!("captures/{source}/{date}/{id}");
        if vfs::stat(&path).is_err() {
            return id;
        }
        index += 1;
    }
}

fn merge_speaker_label(value: &Value) -> Option<String> {
    if value.get("is_user").and_then(Value::as_bool) == Some(true) {
        return Some("You".into());
    }
    value
        .get("speaker")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("speaker")
                .and_then(Value::as_i64)
                .map(|index| format!("SPEAKER_{index:02}"))
        })
}

fn transcript_labels(transcript: &Value, labels: &mut std::collections::BTreeSet<String>) {
    for key in ["segments", "words"] {
        if let Some(rows) = transcript.get(key).and_then(Value::as_array) {
            for row in rows {
                if let Some(label) = merge_speaker_label(row) {
                    labels.insert(label);
                }
            }
        }
    }
}

fn merge_speakers(
    sources: &[MergeSource],
) -> (Vec<std::collections::BTreeMap<String, String>>, Value) {
    let mut maps = Vec::new();
    let mut global = serde_json::Map::new();
    let mut identities = std::collections::BTreeMap::new();
    let mut next = 1usize;
    for source in sources {
        let mut labels = std::collections::BTreeSet::new();
        if let Some(entries) = source.speakers.as_object() {
            labels.extend(entries.keys().cloned());
        }
        transcript_labels(&source.transcript, &mut labels);
        if let Some(items) = source.overrides.as_array() {
            for item in items {
                if let Some(label) = item.get("speaker").and_then(Value::as_str) {
                    labels.insert(label.to_string());
                }
            }
        }
        let mut map = std::collections::BTreeMap::new();
        for label in labels {
            let entry = source
                .speakers
                .get(&label)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let identity = entry
                .get("person_id")
                .and_then(Value::as_str)
                .or_else(|| entry.get("name").and_then(Value::as_str))
                .map(|value| value.trim().to_lowercase())
                .filter(|value| !value.is_empty());
            let mapped = if let Some(identity) = &identity {
                identities.get(identity).cloned()
            } else {
                None
            };
            let mapped = mapped.unwrap_or_else(|| {
                let key = if label == "You" {
                    "You".to_string()
                } else {
                    let key = format!("SPEAKER_{next:02}");
                    next += 1;
                    key
                };
                if !entry.as_object().is_some_and(|entry| entry.is_empty()) {
                    global.insert(key.clone(), entry.clone());
                }
                if let Some(identity) = identity {
                    identities.insert(identity, key.clone());
                }
                key
            });
            if !global.contains_key(&mapped)
                && !entry.as_object().is_some_and(|entry| entry.is_empty())
            {
                global.insert(mapped.clone(), entry);
            }
            map.insert(label, mapped);
        }
        maps.push(map);
    }
    (maps, Value::Object(global))
}

fn remap_transcript_speakers(
    transcript: &mut Value,
    map: &std::collections::BTreeMap<String, String>,
) {
    if let Some(segments) = transcript.get_mut("segments").and_then(Value::as_array_mut) {
        for segment in segments {
            remap_speaker_value(segment, map);
        }
    }
    if let Some(words) = transcript.get_mut("words").and_then(Value::as_array_mut) {
        for word in words {
            remap_speaker_value(word, map);
            if let Some(fields) = word.as_object_mut() {
                if let Some(nested) = fields.get_mut("words").and_then(Value::as_array_mut) {
                    for token in nested {
                        remap_speaker_value(token, map);
                    }
                }
            }
        }
    }
}

fn remap_speaker_value(value: &mut Value, map: &std::collections::BTreeMap<String, String>) {
    let Some(fields) = value.as_object_mut() else {
        return;
    };
    if fields.get("is_user").and_then(Value::as_bool) == Some(true) {
        fields["speaker"] = json!("You");
        return;
    }
    if let Some(label) = fields.get("speaker").and_then(Value::as_str) {
        if let Some(mapped) = map.get(label) {
            fields["speaker"] = json!(mapped);
        }
    } else if let Some(index) = fields.get("speaker").and_then(Value::as_i64) {
        let label = format!("SPEAKER_{index:02}");
        if let Some(mapped) = map.get(&label) {
            fields["speaker"] = json!(mapped);
        }
    }
}

fn offset_transcript(transcript: &mut Value, seconds: f64) {
    if seconds.abs() < f64::EPSILON {
        return;
    }
    for key in ["segments", "words"] {
        if let Some(rows) = transcript.get_mut(key).and_then(Value::as_array_mut) {
            for row in rows {
                offset_time_fields(row, seconds);
            }
        }
    }
}

fn offset_time_fields(value: &mut Value, seconds: f64) {
    let Some(fields) = value.as_object_mut() else {
        return;
    };
    for key in ["start", "end"] {
        if let Some(value) = fields.get(key).and_then(Value::as_f64) {
            fields[key] = json!(value + seconds);
        }
    }
    if let Some(words) = fields.get_mut("words").and_then(Value::as_array_mut) {
        for word in words {
            offset_time_fields(word, seconds);
        }
    }
}

fn read_wav(path: &str) -> Option<WavAudio> {
    let entry = vfs::stat(path).ok()?;
    if entry.size < 44 {
        return None;
    }
    let bytes = read_vfs_bytes(path, 0, entry.size).ok()?;
    let data_at = bytes.windows(4).position(|window| window == b"data")? + 8;
    let sample_rate = u32::from_le_bytes(bytes.get(24..28)?.try_into().ok()?);
    let channels = u16::from_le_bytes(bytes.get(22..24)?.try_into().ok()?);
    let bits = u16::from_le_bytes(bytes.get(34..36)?.try_into().ok()?);
    if sample_rate == 0 || channels == 0 || bits != 16 {
        return None;
    }
    Some(WavAudio {
        sample_rate,
        channels,
        bits,
        pcm: bytes.get(data_at..)?.to_vec(),
    })
}

fn merge_audio(sources: &[MergeSource], earliest: u64) -> Option<Vec<u8>> {
    let first = sources.iter().find_map(|source| source.audio.as_ref())?;
    let sample_rate = first.sample_rate;
    let channels = first.channels;
    let bytes_per_ms = sample_rate as usize * channels as usize * 2 / 1000;
    let mut pcm = Vec::new();
    let mut cursor_ms = 0.0f64;
    for source in sources {
        let offset_ms = source.started_at_ms.saturating_sub(earliest) as f64;
        let Some(audio) = &source.audio else {
            continue;
        };
        if audio.sample_rate != sample_rate || audio.channels != channels || audio.bits != 16 {
            continue;
        }
        if offset_ms > cursor_ms {
            let gap = ((offset_ms - cursor_ms).min(300_000.0) * bytes_per_ms as f64) as usize;
            pcm.resize(pcm.len() + gap, 0);
        }
        pcm.extend_from_slice(&audio.pcm);
        cursor_ms = offset_ms + (audio.pcm.len() / bytes_per_ms.max(1)) as f64;
    }
    if pcm.is_empty() {
        return None;
    }
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    wav.extend_from_slice(&(channels * 2).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    Some(wav)
}

fn watch(args: Args) -> Result<(), String> {
    let started = now_ms()?;
    let mut cursor: Option<String> = None;
    let mut seen = 0_u64;
    let mut stored = 0_u64;
    let mut failed = 0_u64;
    loop {
        let page = bus::poll(&["omi.capture.v1".into()], cursor.as_deref(), 32)
            .map_err(|error| format!("unibus poll: {error:?}"))?;
        for frame in page.frames {
            seen += 1;
            let body: Value = match serde_json::from_slice(&frame.body) {
                Ok(body) => body,
                Err(error) => {
                    failed += 1;
                    eprintln!("scribe: unibus body JSON: {error}");
                    continue;
                }
            };
            match process_event(
                &body,
                &args.model,
                false,
                true,
                &args.deep_model,
                args.deep_after_chars,
            ) {
                Ok(_) => stored += 1,
                Err(error) => {
                    failed += 1;
                    eprintln!("scribe: {error}");
                }
            }
        }
        cursor = page.next;
        if args.once || now_ms()?.saturating_sub(started) >= args.hold_ms {
            break;
        }
        let _ = time::tick(250);
    }
    finish(json!({
        "ok": true,
        "command": "watch",
        "seen": seen,
        "stored": stored,
        "failed": failed,
        "hold_ms": args.hold_ms,
    }));
    Ok(())
}

fn process_event(
    body: &Value,
    model: &str,
    enrich: bool,
    publish_stored: bool,
    deep_model: &str,
    deep_after_chars: usize,
) -> Result<Value, String> {
    let mut event = parse_event(body)?;
    let source = sanitize(&event.source)?;
    let date = normalize_date(event.date.as_deref());
    let id = sanitize(&event.capture_id)?;
    let root = format!("captures/{source}/{date}/{id}");
    mkdir_recursive(&root)?;

    let supplied_summary = event.summary.trim().to_string();
    let supplied_actions = event.actions.clone();
    let supplied_questions = event.questions.clone();
    let supplied_calendar = event.calendar.clone();
    let mut generated_summary: Option<String> = None;
    let mut generated_actions: Option<Value> = None;
    let mut generated_questions: Option<Value> = None;
    let mut generated_calendar: Option<Value> = None;
    let mut enrichment_model = "provided".to_string();
    let mut enrichment_status = if supplied_summary.is_empty() {
        "pending"
    } else {
        "provided"
    };
    if enrich {
        match enrich_capture(&event, model, deep_model, deep_after_chars, false) {
            Ok(enriched) => {
                generated_summary = Some(enriched.summary);
                generated_actions = Some(enriched.actions);
                generated_questions = Some(enriched.questions);
                generated_calendar = Some(enriched.calendar);
                enrichment_model = enriched.model;
                enrichment_status = "complete";
            }
            Err(error) => {
                eprintln!(
                    "scribe: enrichment pending for {}: {error}",
                    event.capture_id
                );
            }
        }
    }
    if let Some(summary) = generated_summary.as_ref() {
        event.summary = summary.clone();
    }
    if let Some(actions) = generated_actions.as_ref() {
        event.actions = actions.clone();
    }
    if let Some(questions) = generated_questions.as_ref() {
        event.questions = questions.clone();
    }
    if let Some(calendar) = generated_calendar.as_ref() {
        event.calendar = calendar.clone();
    }
    let summary_source = if generated_summary.is_some() {
        "scribe"
    } else if !supplied_summary.is_empty() {
        "omi"
    } else {
        "empty"
    };
    let transcript_bytes = serde_json::to_vec_pretty(&event.transcript)
        .map_err(|error| format!("transcript JSON: {error}"))?;
    let summary_bytes = event.summary.as_bytes().to_vec();
    let actions_bytes = serde_json::to_vec_pretty(&event.actions)
        .map_err(|error| format!("actions JSON: {error}"))?;
    let questions_bytes = serde_json::to_vec_pretty(&event.questions)
        .map_err(|error| format!("questions JSON: {error}"))?;
    let calendar_bytes = serde_json::to_vec_pretty(&event.calendar)
        .map_err(|error| format!("calendar JSON: {error}"))?;
    let transcript = write_file(
        &format!("{root}/transcript.json"),
        &transcript_bytes,
        "application/json",
    )?;
    let summary = write_file(
        &format!("{root}/summary.md"),
        &summary_bytes,
        "text/markdown",
    )?;
    let actions = write_file(
        &format!("{root}/actions.json"),
        &actions_bytes,
        "application/json",
    )?;
    let questions = write_file(
        &format!("{root}/questions.json"),
        &questions_bytes,
        "application/json",
    )?;
    let calendar = write_file(
        &format!("{root}/calendar.json"),
        &calendar_bytes,
        "application/json",
    )?;
    let omi_summary = if supplied_summary.is_empty() {
        None
    } else {
        Some(write_file(
            &format!("{root}/summary.omi.md"),
            supplied_summary.as_bytes(),
            "text/markdown",
        )?)
    };
    let scribe_summary = if let Some(summary) = generated_summary.as_ref() {
        Some(write_file(
            &format!("{root}/summary.scribe.md"),
            summary.as_bytes(),
            "text/markdown",
        )?)
    } else {
        None
    };
    let _omi_actions = if supplied_actions
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        Some(write_file(
            &format!("{root}/actions.omi.json"),
            &serde_json::to_vec_pretty(&supplied_actions)
                .map_err(|error| format!("Omi actions JSON: {error}"))?,
            "application/json",
        )?)
    } else {
        None
    };
    let _omi_questions = if supplied_questions
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        Some(write_file(
            &format!("{root}/questions.omi.json"),
            &serde_json::to_vec_pretty(&supplied_questions)
                .map_err(|error| format!("Omi questions JSON: {error}"))?,
            "application/json",
        )?)
    } else {
        None
    };
    let _omi_calendar = if supplied_calendar
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        Some(write_file(
            &format!("{root}/calendar.omi.json"),
            &serde_json::to_vec_pretty(&supplied_calendar)
                .map_err(|error| format!("Omi calendar JSON: {error}"))?,
            "application/json",
        )?)
    } else {
        None
    };
    let summary_index = json!({
        "display": summary_source,
        "omi": omi_summary.as_ref().map(|file| json!({
            "path": file.path,
            "content_id": file.content_id,
        })),
        "scribe": scribe_summary.as_ref().map(|file| json!({
            "path": file.path,
            "content_id": file.content_id,
            "model": enrichment_model,
        })),
    });
    let _summary_index = write_file(
        &format!("{root}/summary.json"),
        &serde_json::to_vec_pretty(&summary_index)
            .map_err(|error| format!("summary index JSON: {error}"))?,
        "application/json",
    )?;
    let title = short_title(
        &summary_title(&event.summary)
            .or_else(|| event.title.clone())
            .unwrap_or_else(|| "Untitled capture".to_string()),
        40,
    );
    let emoji = event
        .emoji
        .clone()
        .unwrap_or_else(|| emoji_for(&title, &event.summary).to_string());
    let capture_meta = serde_json::to_vec_pretty(&json!({
        "capture_id": event.capture_id,
        "source": event.source,
        "date": date,
        "title": title,
        "emoji": emoji,
        "summary_source": summary_source,
        "started_at_ms": event.started_at_ms,
        "ended_at_ms": event.ended_at_ms,
    }))
    .map_err(|error| format!("capture meta JSON: {error}"))?;
    let _meta = write_file(
        &format!("{root}/capture.json"),
        &capture_meta,
        "application/json",
    )?;

    let segment_count = event
        .transcript
        .get("segments")
        .and_then(Value::as_array)
        .map(|segments| segments.len())
        .or_else(|| {
            event
                .transcript
                .get("words")
                .and_then(Value::as_array)
                .map(|words| words.len())
        })
        .unwrap_or(0);
    let speaker_count = speaker_count(&event.transcript);

    let record = json!({
        "schema": "fungos.transcript/capture@1",
        "capture_id": event.capture_id,
        "source": event.source,
        "started_at_ms": event.started_at_ms,
        "ended_at_ms": event.ended_at_ms,
        "bundle": root,
        "transcript": {
            "content_id": transcript.content_id,
            "path": transcript.path,
            "media_type": "application/json",
        },
        "summary": {
            "content_id": summary.content_id,
            "path": summary.path,
            "media_type": "text/markdown",
            "source": summary_source,
            "omi_path": omi_summary.as_ref().map(|file| file.path.clone()),
            "scribe_path": scribe_summary.as_ref().map(|file| file.path.clone()),
        },
        "actions": {
            "content_id": actions.content_id,
            "path": actions.path,
            "media_type": "application/json",
            "count": event.actions.as_array().map(Vec::len).unwrap_or(0),
        },
        "questions": {
            "content_id": questions.content_id,
            "path": questions.path,
            "media_type": "application/json",
            "count": event.questions.as_array().map(Vec::len).unwrap_or(0),
        },
        "calendar": {
            "content_id": calendar.content_id,
            "path": calendar.path,
            "media_type": "application/json",
            "count": event.calendar.as_array().map(Vec::len).unwrap_or(0),
        },
        "audio_content_id": event.audio_content_id,
        "segment_count": segment_count,
        "speaker_count": speaker_count,
        "status": if enrichment_status == "complete" || enrichment_status == "provided" {
            "stored"
        } else {
            "stored_enrichment_pending"
        },
        "enrichment": {
            "status": enrichment_status,
            "model": enrichment_model,
        },
        "scribe": SOURCE_SPORE,
    });
    upsert_record(
        "capture",
        &format!("{source}:{}", event.capture_id),
        &record,
    )?;
    let _ = index::refresh();

    if !event.checkpoint && publish_stored {
        let event_body = serde_json::to_vec(&json!({
        "schema": "scribe.capture.stored.v1",
        "capture_id": event.capture_id,
        "source": source,
        "bundle": root,
        "record": format!("capture:{source}:{}", event.capture_id),
        "started_at_ms": event.started_at_ms,
        "ended_at_ms": event.ended_at_ms,
        "segment_count": segment_count,
        "speaker_count": speaker_count,
        "transcript_content_id": transcript.content_id,
        "summary_content_id": summary.content_id,
        "summary_source": summary_source,
        "omi_summary_content_id": omi_summary.as_ref().map(|file| file.content_id.clone()),
        "scribe_summary_content_id": scribe_summary.as_ref().map(|file| file.content_id.clone()),
        "actions_content_id": actions.content_id,
        "action_count": event.actions.as_array().map(Vec::len).unwrap_or(0),
        "questions_content_id": questions.content_id,
        "question_count": event.questions.as_array().map(Vec::len).unwrap_or(0),
        "calendar_content_id": calendar.content_id,
        "calendar_count": event.calendar.as_array().map(Vec::len).unwrap_or(0),
        "enrichment": enrichment_status,
        "model": enrichment_model,
    }))
    .map_err(|error| format!("stored event JSON: {error}"))?;
        let _ = bus::publish(
            "scribe.capture.stored.v1",
            &event_body,
            None,
            7 * 24 * 60 * 60 * 1000,
        );

        let duration_ms = event
            .started_at_ms
            .zip(event.ended_at_ms)
            .map(|(start, end)| end.saturating_sub(start))
            .unwrap_or(0);
        let enriched_body = serde_json::to_vec(&json!({
            "schema": "scribe.capture.enriched.v1",
            "capture_id": event.capture_id,
            "source": source,
            "bundle": root,
            "record": format!("capture:{source}:{}", event.capture_id),
            "started_at_ms": event.started_at_ms,
            "ended_at_ms": event.ended_at_ms,
            "duration_ms": duration_ms,
            "segment_count": segment_count,
            "speaker_count": speaker_count,
            "summary": {
                "content_id": summary.content_id,
                "path": summary.path,
                "model": enrichment_model,
                "source": summary_source,
                "omi_content_id": omi_summary.as_ref().map(|file| file.content_id.clone()),
                "scribe_content_id": scribe_summary.as_ref().map(|file| file.content_id.clone()),
            },
            "actions": {
                "content_id": actions.content_id,
                "path": actions.path,
                "count": event.actions.as_array().map(Vec::len).unwrap_or(0),
            },
            "questions": {
                "content_id": questions.content_id,
                "path": questions.path,
                "count": event.questions.as_array().map(Vec::len).unwrap_or(0),
            },
            "calendar": {
                "content_id": calendar.content_id,
                "path": calendar.path,
                "count": event.calendar.as_array().map(Vec::len).unwrap_or(0),
            },
            "enrichment": enrichment_status,
        }))
        .map_err(|error| format!("enriched event JSON: {error}"))?;
        let _ = bus::publish(
            "scribe.capture.enriched.v1",
            &enriched_body,
            None,
            7 * 24 * 60 * 60 * 1000,
        );

        // One event per extracted action. Consumers (Elmers, Marbles, JARVIS) can
        // decide whether an item is worth acting on; Scribe never creates one.
        if let Some(items) = event.actions.as_array() {
            for (index, item) in items.iter().enumerate() {
                let action_id = format!("action:{source}:{}:{index}", event.capture_id);
                let body = serde_json::to_vec(&json!({
                    "schema": "scribe.action_item.v1",
                    "id": action_id,
                    "capture_id": event.capture_id,
                    "source": source,
                    "bundle": root,
                    "record": format!("capture:{source}:{}", event.capture_id),
                    "started_at_ms": event.started_at_ms,
                    "ended_at_ms": event.ended_at_ms,
                    "kind": item.get("kind").cloned().unwrap_or(Value::Null),
                    "title": item.get("title").cloned().unwrap_or(Value::Null),
                    "owner": item.get("owner").cloned().unwrap_or(Value::Null),
                    "due_at": item.get("due_at").cloned().unwrap_or(Value::Null),
                    "quote": item.get("quote").cloned().unwrap_or(Value::Null),
                    "start_ms": item.get("start_ms").cloned().unwrap_or(Value::Null),
                    "end_ms": item.get("end_ms").cloned().unwrap_or(Value::Null),
                    "speaker": item.get("speaker").cloned().unwrap_or(Value::Null),
                    "confidence": item.get("confidence").cloned().unwrap_or(Value::Null),
                }))
                .map_err(|error| format!("action item event JSON: {error}"))?;
                let _ = bus::publish(
                    "scribe.action_item.v1",
                    &body,
                    None,
                    7 * 24 * 60 * 60 * 1000,
                );
            }
        }
        if let Some(items) = event.questions.as_array() {
            for (index, item) in items.iter().enumerate() {
                let body = serde_json::to_vec(&json!({
                    "schema": "scribe.question.v1",
                    "id": format!("question:{source}:{}:{index}", event.capture_id),
                    "capture_id": event.capture_id,
                    "source": source,
                    "bundle": root,
                    "record": format!("capture:{source}:{}", event.capture_id),
                    "started_at_ms": event.started_at_ms,
                    "ended_at_ms": event.ended_at_ms,
                    "question": item.get("question").cloned().unwrap_or(Value::Null),
                    "asked_by": item.get("asked_by").cloned().unwrap_or(Value::Null),
                    "directed_at": item.get("directed_at").cloned().unwrap_or_else(|| json!([])),
                    "blocking": item.get("blocking").cloned().unwrap_or(Value::Bool(false)),
                    "quote": item.get("quote").cloned().unwrap_or(Value::Null),
                    "start_ms": item.get("start_ms").cloned().unwrap_or(Value::Null),
                    "speaker": item.get("speaker").cloned().unwrap_or(Value::Null),
                    "confidence": item.get("confidence").cloned().unwrap_or(Value::Null),
                }))
                .map_err(|error| format!("question event JSON: {error}"))?;
                let _ = bus::publish("scribe.question.v1", &body, None, 7 * 24 * 60 * 60 * 1000);
            }
        }
        if let Some(items) = event.calendar.as_array() {
            for (index, item) in items.iter().enumerate() {
                let body = serde_json::to_vec(&json!({
                    "schema": "scribe.calendar_candidate.v1",
                    "id": format!("calendar:{source}:{}:{index}", event.capture_id),
                    "capture_id": event.capture_id,
                    "source": source,
                    "bundle": root,
                    "record": format!("capture:{source}:{}", event.capture_id),
                    "started_at_ms": event.started_at_ms,
                    "ended_at_ms": event.ended_at_ms,
                    "title": item.get("title").cloned().unwrap_or(Value::Null),
                    "start": item.get("start").cloned().unwrap_or(Value::Null),
                    "end": item.get("end").cloned().unwrap_or(Value::Null),
                    "timezone": item.get("timezone").cloned().unwrap_or(Value::Null),
                    "recurrence": item.get("recurrence").cloned().unwrap_or(Value::Null),
                    "location": item.get("location").cloned().unwrap_or(Value::Null),
                    "join_url": item.get("join_url").cloned().unwrap_or(Value::Null),
                    "status": item.get("status").cloned().unwrap_or_else(|| json!("hold")),
                    "visibility": item.get("visibility").cloned().unwrap_or_else(|| json!("personal")),
                    "organizer": item.get("organizer").cloned().unwrap_or(Value::Null),
                    "attendees": item.get("attendees").cloned().unwrap_or_else(|| json!([])),
                    "quote": item.get("quote").cloned().unwrap_or(Value::Null),
                    "start_ms": item.get("start_ms").cloned().unwrap_or(Value::Null),
                    "confidence": item.get("confidence").cloned().unwrap_or(Value::Null),
                }))
                .map_err(|error| format!("calendar event JSON: {error}"))?;
                let _ = bus::publish(
                    "scribe.calendar_candidate.v1",
                    &body,
                    None,
                    7 * 24 * 60 * 60 * 1000,
                );
            }
        }
    }

    Ok(json!({
        "capture_id": event.capture_id,
        "record": format!("capture:{source}:{}", event.capture_id),
        "bundle": root,
        "segments": segment_count,
        "speakers": speaker_count,
        "enrichment": enrichment_status,
    }))
}

#[derive(Debug)]
struct Enriched {
    summary: String,
    actions: Value,
    questions: Value,
    calendar: Value,
    model: String,
}

fn enrich_capture(
    event: &CaptureEvent,
    model: &str,
    deep_model: &str,
    deep_after_chars: usize,
    design: bool,
) -> Result<Enriched, String> {
    let text = transcript_text(&event.transcript);
    if text.trim().is_empty() {
        return Err("transcript has no text".into());
    }
    let use_deep = text.chars().count() > deep_after_chars;
    let selected = if use_deep && !deep_model.trim().is_empty() {
        deep_model
    } else {
        model
    };
    if use_deep && deep_model.trim().is_empty() {
        return Err("transcript too large; deep summary deferred".into());
    }
    // A larger-context model can absorb substantially more of the meeting in
    // one pass; the 9B keeps its existing bounded chunks.
    let chunk_chars = if use_deep { 120_000 } else { 24_000 };
    let chunks = split_chunks(&text, chunk_chars, 500);
    let max_output_tokens = if use_deep { 16_384 } else { 2_048 };
    let mut summaries = Vec::new();
    let mut actions: Vec<Value> = Vec::new();
    let mut questions: Vec<Value> = Vec::new();
    let mut calendar: Vec<Value> = Vec::new();
    let mut model_used = model.to_string();

    for (index, chunk) in chunks.iter().enumerate() {
        let request = inference::Request {
            model: selected.to_string(),
            messages: vec![
                inference::Message {
                    role: inference::Role::System,
                    content: Some(if design {
                        "You are Scribe in technical design-doc mode. Return ONLY valid JSON.\n                         Schema: {\"summary_md\": string, \"actions\": array, \"questions\": array, \"calendar\": array}.\n                         summary_md MUST begin with one Markdown H1 heading of 2-5 words naming the concrete technical subject. Then use these sections with bullets where possible: ## Context, ## Current design, ## Decisions, ## Open questions, ## Next steps. Preserve exact APIs, schemas, model names, file paths, constraints, and numbers. Prefer architecture/decision language over a narrative recap. Do not invent facts.\n                         Each action: {\"kind\": \"task|feature_request|decision|commitment\", \"title\": string, \"owner\": string, \"due_at\": string|null, \"quote\": string, \"start_ms\": number|null, \"end_ms\": number|null, \"speaker\": string|null, \"confidence\": number}.\n                         Each question: {\"question\": string, \"asked_by\": string|null, \"directed_at\": [string], \"blocking\": boolean, \"quote\": string, \"start_ms\": number|null, \"end_ms\": number|null, \"speaker\": string|null, \"confidence\": number}.\n                         Each calendar candidate: {\"title\": string, \"start\": string|null, \"end\": string|null, \"timezone\": string|null, \"recurrence\": string|null, \"location\": string|null, \"join_url\": string|null, \"status\": \"hold|tentative|confirmed\", \"visibility\": \"personal|attendees\", \"organizer\": string|null, \"attendees\": [{\"person_id\": string|null, \"display_name\": string, \"email\": string|null, \"role\": \"required|optional|fyi\", \"source\": \"present|named|explicit_email\", \"confidence\": number}], \"quote\": string, \"start_ms\": number|null, \"confidence\": number}.\n                         Rules: every action, question and calendar candidate must quote the transcript exactly. Questions are things to resolve, not tasks. Calendar candidates are proposals, never sent invites. Do not invent contacts, emails, dates, or times; use null when unknown. If no action exists return an empty array."
                    } else {
                        "You are Scribe, a meeting transcript extractor. Return ONLY valid JSON.\n                         Schema: {\"summary_md\": string, \"actions\": array, \"questions\": array, \"calendar\": array}.\n                         summary_md MUST begin with one Markdown H1 heading of 2-5 words naming the meeting's concrete subject. Do not use generic words like 'meeting' or 'discussion'. Then write a concise overview, decisions, and open questions.\n                         Each action: {\"kind\": \"task|feature_request|decision|commitment\",                          \"title\": string, \"owner\": string, \"due_at\": string|null,                          \"quote\": string, \"start_ms\": number|null, \"end_ms\": number|null,                          \"speaker\": string|null, \"confidence\": number}.\n                         Each question: {\"question\": string, \"asked_by\": string|null, \"directed_at\": [string], \"blocking\": boolean, \"quote\": string, \"start_ms\": number|null, \"end_ms\": number|null, \"speaker\": string|null, \"confidence\": number}.\n                         Each calendar candidate: {\"title\": string, \"start\": string|null, \"end\": string|null, \"timezone\": string|null, \"recurrence\": string|null, \"location\": string|null, \"join_url\": string|null, \"status\": \"hold|tentative|confirmed\", \"visibility\": \"personal|attendees\", \"organizer\": string|null, \"attendees\": [{\"person_id\": string|null, \"display_name\": string, \"email\": string|null, \"role\": \"required|optional|fyi\", \"source\": \"present|named|explicit_email\", \"confidence\": number}], \"quote\": string, \"start_ms\": number|null, \"confidence\": number}.\n                         Rules: every action, question and calendar candidate must quote the transcript exactly. Questions are things to resolve, not tasks. Calendar candidates are proposals, never sent invites. Do not invent contacts, emails, dates, or times; use null when unknown. Do not invent owners, dates, or tasks. If no action exists return an empty array."
                    }.into()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                    attachments: vec![],
                    reasoning: None,
                },
                inference::Message {
                    role: inference::Role::User,
                    content: Some(format!(
                        "MEETING TRANSCRIPT part {}/{}:\n\n{}",
                        index + 1,
                        chunks.len(),
                        chunk
                    )),
                    name: None,
                    tool_call_id: None,
                    tool_calls: vec![],
                    attachments: vec![],
                    reasoning: None,
                },
            ],
            tools: vec![],
            max_output_tokens: Some(max_output_tokens),
            temperature: Some(0.2),
            prompt_cache_key: Some(format!("scribe:{}", event.capture_id)),
        };
        let response =
            inference::complete(&request).map_err(|error| format!("inference: {error:?}"))?;
        model_used = response.model.clone();
        let content = response
            .message
            .content
            .as_deref()
            .ok_or_else(|| "inference returned no content".to_string())?;
        let value = parse_model_json(content)?;
        summaries.push(
            value
                .get("summary_md")
                .or_else(|| value.get("summary"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
        );
        if let Some(items) = value.get("actions").and_then(Value::as_array) {
            for item in items {
                let title = item
                    .get("title")
                    .or_else(|| item.get("task"))
                    .or_else(|| item.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let quote = item.get("quote").and_then(Value::as_str).unwrap_or("");
                if title.is_empty() && quote.is_empty() {
                    continue;
                }
                let mut item = item.clone();
                item["title"] = json!(title);
                if item.get("due_at").is_none() {
                    if let Some(due) = item.get("due").cloned() {
                        item["due_at"] = due;
                    }
                }
                if !actions.iter().any(|existing| {
                    existing.get("title").and_then(Value::as_str) == Some(title)
                        && existing.get("quote").and_then(Value::as_str) == Some(quote)
                }) {
                    actions.push(item);
                }
            }
        }
        if let Some(items) = value.get("questions").and_then(Value::as_array) {
            for item in items {
                let question = item
                    .get("question")
                    .or_else(|| item.get("title"))
                    .or_else(|| item.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let quote = item.get("quote").and_then(Value::as_str).unwrap_or("");
                if question.is_empty() || quote.is_empty() {
                    continue;
                }
                let mut item = item.clone();
                item["question"] = json!(question);
                item["directed_at"] = item
                    .get("directed_at")
                    .cloned()
                    .filter(Value::is_array)
                    .unwrap_or_else(|| json!([]));
                item["blocking"] = json!(item
                    .get("blocking")
                    .and_then(Value::as_bool)
                    .unwrap_or(false));
                if !questions.iter().any(|existing| {
                    existing.get("question").and_then(Value::as_str) == Some(question)
                        && existing.get("quote").and_then(Value::as_str) == Some(quote)
                }) {
                    questions.push(item);
                }
            }
        }
        if let Some(items) = value.get("calendar").and_then(Value::as_array) {
            for item in items {
                let title = item
                    .get("title")
                    .or_else(|| item.get("summary"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let quote = item.get("quote").and_then(Value::as_str).unwrap_or("");
                if title.is_empty() || quote.is_empty() {
                    continue;
                }
                let mut item = item.clone();
                item["title"] = json!(title);
                item["status"] = json!(match item.get("status").and_then(Value::as_str) {
                    Some("confirmed") => "confirmed",
                    Some("tentative") => "tentative",
                    _ => "hold",
                });
                item["visibility"] = json!(match item.get("visibility").and_then(Value::as_str) {
                    Some("attendees") => "attendees",
                    _ => "personal",
                });
                item["attendees"] = item
                    .get("attendees")
                    .and_then(Value::as_array)
                    .map(|attendees| {
                        Value::Array(
                            attendees
                                .iter()
                                .filter_map(|attendee| {
                                    let name = attendee
                                        .get("display_name")
                                        .or_else(|| attendee.get("name"))
                                        .or_else(|| attendee.get("person_id"))
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .trim();
                                    if name.is_empty() {
                                        return None;
                                    }
                                    let mut attendee = attendee.clone();
                                    attendee["display_name"] = json!(name);
                                    attendee["role"] =
                                        json!(match attendee.get("role").and_then(Value::as_str) {
                                            Some("optional") => "optional",
                                            Some("fyi") => "fyi",
                                            _ => "required",
                                        });
                                    attendee["source"] = json!(match attendee
                                        .get("source")
                                        .and_then(Value::as_str)
                                    {
                                        Some("named") => "named",
                                        Some("explicit_email") => "explicit_email",
                                        _ => "present",
                                    });
                                    attendee["confidence"] = json!(attendee
                                        .get("confidence")
                                        .and_then(Value::as_f64)
                                        .unwrap_or(0.5));
                                    Some(attendee)
                                })
                                .collect(),
                        )
                    })
                    .unwrap_or_else(|| json!([]));
                let start = item.get("start").and_then(Value::as_str).unwrap_or("");
                let duplicate = calendar.iter().any(|existing| {
                    existing.get("title").and_then(Value::as_str) == Some(title)
                        && existing.get("start").and_then(Value::as_str).unwrap_or("") == start
                });
                if !duplicate {
                    calendar.push(item);
                }
            }
        }
    }

    let summary = summaries
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    Ok(Enriched {
        summary,
        actions: Value::Array(actions),
        questions: Value::Array(questions),
        calendar: Value::Array(calendar),
        model: model_used,
    })
}

fn transcript_text(transcript: &Value) -> String {
    if let Some(segments) = transcript.get("segments").and_then(Value::as_array) {
        let lines: Vec<String> = segments
            .iter()
            .filter_map(|segment| {
                let text = segment.get("text").and_then(Value::as_str)?;
                let speaker = speaker_label(segment.get("speaker"));
                let start = segment.get("start").and_then(Value::as_f64).unwrap_or(0.0);
                let end = segment.get("end").and_then(Value::as_f64).unwrap_or(start);
                Some(format!("[{start:.1}-{end:.1}] {speaker}: {text}"))
            })
            .collect();
        if !lines.is_empty() {
            return lines.join("\n");
        }
    }
    if let Some(words) = transcript.get("words").and_then(Value::as_array) {
        let mut lines = Vec::new();
        let mut current_speaker = String::new();
        let mut current_text = String::new();
        let mut current_start = 0.0_f64;
        let mut current_end = 0.0_f64;
        for word in words {
            let speaker = speaker_label(word.get("speaker"));
            let text = word.get("word").and_then(Value::as_str).unwrap_or("");
            let start = word
                .get("start")
                .and_then(Value::as_f64)
                .unwrap_or(current_end);
            let end = word.get("end").and_then(Value::as_f64).unwrap_or(start);
            if speaker != current_speaker {
                if !current_text.trim().is_empty() {
                    lines.push(format!(
                        "[{current_start:.1}-{current_end:.1}] {current_speaker}: {}",
                        current_text.trim()
                    ));
                }
                current_speaker = speaker;
                current_text.clear();
                current_start = start;
            }
            current_end = end;
            if !current_text.is_empty() {
                current_text.push(' ');
            }
            current_text.push_str(text);
        }
        if !current_text.trim().is_empty() {
            lines.push(format!(
                "[{current_start:.1}-{current_end:.1}] {current_speaker}: {}",
                current_text.trim()
            ));
        }
        if !lines.is_empty() {
            return lines.join("\n");
        }
    }
    transcript
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn speaker_label(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .map(|number| format!("SPEAKER_{number:02}"))
            .unwrap_or_else(|| "SPEAKER_00".into()),
        Some(Value::String(label)) => label.trim_matches('"').to_string(),
        _ => "SPEAKER_00".into(),
    }
}

fn split_chunks(text: &str, max_chars: usize, overlap: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_chars).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end < text.len() {
            if let Some(last_newline) = text[start..end].rfind('\n') {
                if last_newline > max_chars / 2 {
                    end = start + last_newline;
                }
            }
        }
        chunks.push(text[start..end].to_string());
        if end >= text.len() {
            break;
        }
        start = end.saturating_sub(overlap);
        while start < text.len() && !text.is_char_boundary(start) {
            start += 1;
        }
    }
    chunks
}

fn parse_model_json(content: &str) -> Result<Value, String> {
    let trimmed = content.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed).trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    let start = trimmed
        .find('{')
        .ok_or_else(|| "model output has no JSON object".to_string())?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| "model output has no JSON object".to_string())?;
    serde_json::from_str(&trimmed[start..=end]).map_err(|error| format!("model JSON: {error}"))
}

#[derive(Debug)]
struct CaptureEvent {
    capture_id: String,
    source: String,
    date: Option<String>,
    started_at_ms: Option<u64>,
    ended_at_ms: Option<u64>,
    transcript: Value,
    summary: String,
    actions: Value,
    questions: Value,
    calendar: Value,
    audio_content_id: Option<String>,
    title: Option<String>,
    emoji: Option<String>,
    checkpoint: bool,
}

fn parse_event(body: &Value) -> Result<CaptureEvent, String> {
    let value = body
        .get("body")
        .or_else(|| body.get("event"))
        .unwrap_or(body);
    let capture_id = value
        .get("capture_id")
        .or_else(|| value.get("conversation_id"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| "capture event needs capture_id".to_string())?
        .to_string();
    let source = value
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("omi")
        .to_string();
    let date = value
        .get("date")
        .and_then(Value::as_str)
        .map(str::to_string);
    let started_at_ms = value.get("started_at_ms").and_then(Value::as_u64);
    let ended_at_ms = value.get("ended_at_ms").and_then(Value::as_u64);
    let transcript = match value.get("transcript") {
        Some(Value::String(text)) => json!({ "text": text }),
        Some(value) => value.clone(),
        None => value
            .get("transcript_json")
            .and_then(Value::as_str)
            .map(|text| serde_json::from_str(text).unwrap_or_else(|_| json!({ "text": text })))
            .ok_or_else(|| "capture event needs transcript".to_string())?,
    };
    let summary = value
        .get("summary_md")
        .or_else(|| value.get("summary"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let actions = value.get("actions").cloned().unwrap_or_else(|| json!([]));
    let questions = value.get("questions").cloned().unwrap_or_else(|| json!([]));
    let calendar = value
        .get("calendar")
        .or_else(|| value.get("calendar_candidates"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let audio_content_id = value
        .get("audio_content_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("omi")
                .and_then(|omi| omi.get("title"))
                .and_then(Value::as_str)
        })
        .map(str::to_string);
    let emoji = value
        .get("emoji")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|emoji| !emoji.trim().is_empty());
    let checkpoint = value
        .get("checkpoint")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(CaptureEvent {
        capture_id,
        source,
        date,
        started_at_ms,
        ended_at_ms,
        transcript,
        summary,
        actions,
        questions,
        calendar,
        audio_content_id,
        title,
        emoji,
        checkpoint,
    })
}

fn emoji_for(title: &str, summary: &str) -> &'static str {
    let haystack = format!("{title} {summary}").to_lowercase();
    let rules: &[(&[&str], &str)] = &[
        (&["partner", "partnership", "collab", "alliance"], "🤝"),
        (&["launch", "release", "ship", "announce"], "🚀"),
        (&["bug", "debug", "incident", "outage", "fix"], "🐛"),
        (&["design", "ui", "ux", "visual", "style"], "🎨"),
        (
            &["architecture", "system", "infra", "platform", "sdk"],
            "🧱",
        ),
        (&["finance", "revenue", "pricing", "cost", "budget"], "💰"),
        (&["hire", "interview", "candidate", "team"], "🧑‍💼"),
        (&["research", "experiment", "test", "benchmark"], "🧪"),
        (&["plan", "roadmap", "strategy", "priority"], "🗺️"),
        (&["meeting", "sync", "standup", "call"], "📞"),
        (&["health", "workout", "sleep", "run"], "🫀"),
        (&["travel", "trip", "flight", "drive"], "✈️"),
        (&["food", "lunch", "dinner", "recipe"], "🍽️"),
    ];
    for (needles, emoji) in rules {
        if needles.iter().any(|needle| haystack.contains(needle)) {
            return emoji;
        }
    }
    "💬"
}

fn short_title(value: &str, max_chars: usize) -> String {
    let line = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .trim_start_matches('#')
        .trim()
        .trim_start_matches("Overview:")
        .trim();
    let first = line
        .split(". ")
        .next()
        .unwrap_or(line)
        .trim_end_matches('.');
    let chars: Vec<char> = first.chars().collect();
    if chars.len() <= max_chars {
        return first.trim().to_string();
    }
    let cut = chars[..max_chars]
        .iter()
        .rposition(|ch| ch.is_whitespace())
        .unwrap_or(max_chars);
    let trimmed: String = chars[..cut]
        .iter()
        .collect::<String>()
        .trim_end_matches(|ch: char| ch.is_whitespace() || ",;:.-—".contains(ch))
        .to_string();
    format!("{trimmed}…")
}

fn summary_title(summary: &str) -> Option<String> {
    summary
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| short_title(line, 40))
        .filter(|line| !line.is_empty())
}

fn speaker_count(transcript: &Value) -> usize {
    let mut speakers = std::collections::BTreeSet::new();
    for key in ["words", "segments"] {
        if let Some(rows) = transcript.get(key).and_then(Value::as_array) {
            for row in rows {
                if let Some(speaker) = row.get("speaker") {
                    speakers.insert(speaker.to_string());
                }
            }
        }
    }
    speakers.len()
}

fn sanitize(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty id".into());
    }
    if value.len() > 128 {
        return Err("id too long".into());
    }
    if value
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!("id {value:?} has unsupported characters"));
    }
    Ok(value.to_string())
}

fn normalize_date(date: Option<&str>) -> String {
    let Some(date) = date else {
        return "undated".into();
    };
    let bytes = date.as_bytes();
    let valid = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit());
    if valid {
        date.to_string()
    } else {
        "undated".into()
    }
}

fn mkdir_recursive(path: &str) -> Result<(), String> {
    let mut prefix = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        match vfs::stat(&prefix) {
            Ok(entry) if entry.kind == soil::vfs::vfs::Kind::Directory => {}
            Ok(_) => return Err(format!("{prefix} is not a directory")),
            Err(_) => match vfs_mut::mkdir(&prefix) {
                Ok(()) => {}
                Err(vfs_mut::Error::AlreadyExists) => {
                    let entry =
                        vfs::stat(&prefix).map_err(|error| format!("stat {prefix}: {error:?}"))?;
                    if entry.kind != soil::vfs::vfs::Kind::Directory {
                        return Err(format!("{prefix} is not a directory"));
                    }
                }
                Err(error) => return Err(format!("mkdir {prefix}: {error:?}")),
            },
        }
    }
    Ok(())
}

struct Written {
    content_id: String,
    path: String,
}

fn write_file(path: &str, bytes: &[u8], media_type: &str) -> Result<Written, String> {
    let result = vfs_mut::write(path, bytes, media_type, None, None)
        .map_err(|error| format!("write {path}: {error:?}"))?;
    Ok(Written {
        content_id: result.content_id,
        path: path.to_string(),
    })
}

fn upsert_record(collection: &str, id: &str, body: &Value) -> Result<(), String> {
    let entry = Entry {
        key: Key {
            collection: collection.to_string(),
            id: id.to_string(),
        },
        schema: "fungos.transcript/capture@1".into(),
        revision: 0,
        updated_at_ms: 0,
        origin: String::new(),
        deleted: false,
        body: serde_json::to_vec(body).map_err(|error| format!("record JSON: {error}"))?,
    };
    records::upsert(&entry)
        .map(|_| ())
        .map_err(|error| format!("record {collection}/{id}: {error:?}"))
}

fn now_ms() -> Result<u64, String> {
    time::monotonic_ms().map_err(|error| format!("time: {error:?}"))
}

fn finish(value: Value) {
    SNAPSHOT.with(|slot| *slot.borrow_mut() = value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_ids_are_restricted() {
        assert!(sanitize("omi-abc_123").is_ok());
        assert!(sanitize("../etc").is_err());
        assert!(sanitize("a/b").is_err());
        assert!(sanitize("").is_err());
    }

    #[test]
    fn date_only_accepts_iso_days() {
        assert_eq!(normalize_date(Some("2026-11-19")), "2026-11-19");
        assert_eq!(normalize_date(Some("2026-1-9")), "undated");
        assert_eq!(normalize_date(None), "undated");
    }

    #[test]
    fn counts_speakers_and_words() {
        let transcript = json!({
            "words": [
                {"word": "a", "speaker": 1},
                {"word": "b", "speaker": 2},
                {"word": "c", "speaker": 2}
            ]
        });
        assert_eq!(speaker_count(&transcript), 2);
    }

    #[test]
    fn transcript_text_keeps_speakers_and_times() {
        let transcript = json!({
            "words": [
                {"word": "hello", "start": 0.0, "end": 0.5, "speaker": 1},
                {"word": "there", "start": 0.5, "end": 0.9, "speaker": 1},
                {"word": "hi", "start": 1.0, "end": 1.2, "speaker": 2}
            ]
        });
        let text = transcript_text(&transcript);
        assert!(text.contains("SPEAKER_01"));
        assert!(text.contains("SPEAKER_02"));
        assert!(text.contains("[0.0-0.9]"));
    }

    #[test]
    fn speaker_windows_group_words_into_turns() {
        let transcript = json!({
            "words": [
                {"word": "hello", "start": 0.0, "end": 0.5, "speaker": 1},
                {"word": "there", "start": 0.5, "end": 1.0, "speaker": 1},
                {"word": "hi", "start": 1.1, "end": 1.5, "speaker": 2}
            ]
        });
        let windows = speaker_windows(&transcript);
        let (start, end) = windows["SPEAKER_01"];
        assert!(start == 0.0 && end == 1.0, "got {start}..{end}");
        assert_eq!(windows["SPEAKER_02"], (1.1, 2.1));
    }

    #[test]
    fn emoji_is_stable_and_title_stays_short() {
        assert_eq!(emoji_for("DFM SDK partnership", ""), "🤝");
        assert_eq!(emoji_for("Incident review", "the outage was fixed"), "🐛");
        let title = short_title(
            "A very long manufacturing partnership title that should never reach the card",
            40,
        );
        assert!(title.chars().count() <= 41, "{title}");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn short_title_clamps_long_legacy_text() {
        let title = short_title(
            "SPEAKER_01 explains a very long manufacturing and software partnership story that should not be a card title",
            40,
        );
        assert!(title.chars().count() <= 41, "{title}");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn model_json_accepts_fences() {
        let value =
            parse_model_json("```json\n{\"summary_md\":\"ok\",\"actions\":[]}\n```").unwrap();
        assert_eq!(value["summary_md"], "ok");
    }

    #[test]
    fn split_chunks_keeps_short_text_whole() {
        assert_eq!(split_chunks("hello", 100, 10), vec!["hello".to_string()]);
        let chunks = split_chunks(&"a".repeat(250), 100, 10);
        assert!(chunks.len() >= 3);
    }

    #[test]
    fn event_accepts_inline_transcript_and_summary() {
        let body = json!({
            "capture_id": "omi-123",
            "source": "omi",
            "date": "2026-11-19",
            "transcript": "hello",
            "summary_md": "# Summary",
            "actions": [{"kind": "action", "title": "send it"}]
        });
        let event = parse_event(&body).unwrap();
        assert_eq!(event.capture_id, "omi-123");
        assert_eq!(event.transcript["text"], "hello");
        assert_eq!(event.summary, "# Summary");
        assert_eq!(event.actions.as_array().unwrap().len(), 1);
    }
}
