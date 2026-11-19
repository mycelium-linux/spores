//! Scribe: durable capture bundles from Unibus into VFS + records.
//!
//! The Spore decides the bundle layout and the `fungos.transcript/capture@1`
//! shape. The host owns bytes in VFS, record durability, indexing, and the
//! Unibus transport. Nothing here uses a socket, filesystem path, secret, or
//! wall clock.

use std::cell::RefCell;

use serde_json::{json, Value};
use soil::index::index;
use soil::records::records::{self, Entry, Key};
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
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "watch".into(),
            hold_ms: 5_000,
            once: false,
            event: None,
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
                "--help" | "-h" => {
                    return Err(
                        "scribe commands: watch [--hold-ms N] [--once] | ingest --event JSON"
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
            let stored = process_event(&body)?;
            finish(json!({"ok": true, "command": "ingest", "capture": stored}));
            Ok(())
        }
        "watch" => watch(args),
        other => Err(format!("unknown command {other}")),
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
            match process_event(&body) {
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

fn process_event(body: &Value) -> Result<Value, String> {
    let event = parse_event(body)?;
    let source = sanitize(&event.source)?;
    let date = normalize_date(event.date.as_deref());
    let id = sanitize(&event.capture_id)?;
    let root = format!("captures/{source}/{date}/{id}");
    mkdir_recursive(&root)?;

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
        "status": "stored",
        "scribe": SOURCE_SPORE,
    });
    upsert_record(
        "capture",
        &format!("{source}:{}", event.capture_id),
        &record,
    )?;
    let _ = index::refresh();

    let event_body = serde_json::to_vec(&json!({
        "schema": "scribe.capture.stored.v1",
        "capture_id": event.capture_id,
        "record": format!("capture:{source}:{}", event.capture_id),
        "transcript_content_id": transcript.content_id,
        "summary_content_id": summary.content_id,
        "actions_content_id": actions.content_id,
        "action_count": event.actions.as_array().map(Vec::len).unwrap_or(0),
    }))
    .map_err(|error| format!("stored event JSON: {error}"))?;
    let _ = bus::publish("scribe.capture.stored.v1", &event_body, None, 60_000);

    Ok(json!({
        "capture_id": event.capture_id,
        "record": format!("capture:{source}:{}", event.capture_id),
        "bundle": root,
        "segments": segment_count,
        "speakers": speaker_count,
    }))
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
}

fn parse_event(body: &Value) -> Result<CaptureEvent, String> {
    let value = body.get("body").unwrap_or(body);
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
    })
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
