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
                "--help" | "-h" => {
                    return Err(
                        "scribe commands: watch [--hold-ms N] [--once] | ingest --event JSON [--model ALIAS] [--no-enrich]"
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
            let stored = process_event(&body, &args.model, args.enrich)?;
            finish(json!({"ok": true, "command": "ingest", "capture": stored}));
            Ok(())
        }
        "watch" => watch(args),
        "receive" => receive(args),
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
    let stored = process_event(&body, &args.model, args.enrich && !checkpoint)?;
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
            match process_event(&body, &args.model, args.enrich) {
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

fn process_event(body: &Value, model: &str, enrich: bool) -> Result<Value, String> {
    let mut event = parse_event(body)?;
    let source = sanitize(&event.source)?;
    let date = normalize_date(event.date.as_deref());
    let id = sanitize(&event.capture_id)?;
    let root = format!("captures/{source}/{date}/{id}");
    mkdir_recursive(&root)?;

    let supplied_summary = event.summary.trim().to_string();
    let supplied_actions = event.actions.clone();
    let mut generated_summary: Option<String> = None;
    let mut generated_actions: Option<Value> = None;
    let mut enrichment_model = "provided".to_string();
    let mut enrichment_status = if supplied_summary.is_empty() {
        "pending"
    } else {
        "provided"
    };
    if enrich {
        match enrich_capture(&event, model) {
            Ok(enriched) => {
                generated_summary = Some(enriched.summary);
                generated_actions = Some(enriched.actions);
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
    let title = event
        .title
        .clone()
        .or_else(|| summary_title(&event.summary))
        .unwrap_or_else(|| "Untitled capture".to_string());
    let capture_meta = serde_json::to_vec_pretty(&json!({
        "capture_id": event.capture_id,
        "source": event.source,
        "date": date,
        "title": title,
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

    if !event.checkpoint {
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
    model: String,
}

fn enrich_capture(event: &CaptureEvent, model: &str) -> Result<Enriched, String> {
    let text = transcript_text(&event.transcript);
    if text.trim().is_empty() {
        return Err("transcript has no text".into());
    }
    let chunks = split_chunks(&text, 24_000, 500);
    let mut summaries = Vec::new();
    let mut actions: Vec<Value> = Vec::new();
    let mut model_used = model.to_string();

    for (index, chunk) in chunks.iter().enumerate() {
        let request = inference::Request {
            model: model.to_string(),
            messages: vec![
                inference::Message {
                    role: inference::Role::System,
                    content: Some(
                        "You are Scribe, a meeting transcript extractor. Return ONLY valid JSON.\n                         Schema: {\"summary_md\": string, \"actions\": array}.\n                         Each action: {\"kind\": \"action|reminder|feature_request|decision|open_question|commitment\",                          \"title\": string, \"owner\": string, \"due_at\": string|null,                          \"quote\": string, \"start_ms\": number|null, \"end_ms\": number|null,                          \"speaker\": string|null, \"confidence\": number}.\n                         Rules: every action must quote the transcript exactly. Do not invent owners, dates, or tasks.                          If no action exists return an empty array. Keep summary_md concise with overview, decisions, and open questions."
                            .into(),
                    ),
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
            max_output_tokens: Some(2048),
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
                let title = item.get("title").and_then(Value::as_str).unwrap_or("");
                let quote = item.get("quote").and_then(Value::as_str).unwrap_or("");
                if title.is_empty() && quote.is_empty() {
                    continue;
                }
                if !actions.iter().any(|existing| {
                    existing.get("title").and_then(Value::as_str) == Some(title)
                        && existing.get("quote").and_then(Value::as_str) == Some(quote)
                }) {
                    actions.push(item.clone());
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
    audio_content_id: Option<String>,
    title: Option<String>,
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
        audio_content_id,
        title,
        checkpoint,
    })
}

fn summary_title(summary: &str) -> Option<String> {
    summary
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
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
            Err(_) => {
                vfs_mut::mkdir(&prefix).map_err(|error| format!("mkdir {prefix}: {error:?}"))?;
            }
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
