//! Scribe UI: a browser view over the capture bundles Scribe writes.
//!
//! The Spore owns the HTML, the list/detail shape, and the transcript grouping.
//! The host owns routing, VFS bytes and the VFS index. No filesystem path is
//! ever handed to the guest: every read is a workspace-relative `soil:vfs`
//! path.

wit_bindgen::generate!({
    path: "wit",
    world: "scribe-ui",
    generate_all,
});

use serde_json::{json, Value};
use wasi::http::types::{
    Fields, IncomingRequest, Method, OutgoingBody, OutgoingResponse, ResponseOutparam,
};

use soil::vfs::vfs;
use soil::vfs_mut::vfs_mut;

const HTML: &str = include_str!("ui.html");
const MAX_READ: u32 = 1024 * 1024;
const MIN_CAPTURE_WORDS: usize = 20;
const LOW_SIGNAL_TITLES: &[&str] = &[
    "work",
    "working",
    "uma",
    "a quarter mention",
    "god damn",
    "certainly, i'm not sure.",
    "is this thing working?",
    "trying something exciting",
    "a brief multilingual sign-off",
    "alexa lights turn on",
    "measuring tool at the lab",
    "unclear spinach remark",
];

struct Component;

export!(Component);

impl exports::wasi::http::incoming_handler::Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();
        let target = request
            .path_with_query()
            .unwrap_or_else(|| "/".to_string());
        let (path, query) = target
            .split_once('?')
            .map(|(path, query)| (path, query))
            .unwrap_or((target.as_str(), ""));

        let (headers, status, body) = match (method, path) {
            (Method::Get, "/") | (Method::Get, "/index.html") => {
                respond(200, "text/html; charset=utf-8", HTML.as_bytes())
            }
            (Method::Get, "/api/captures") => match captures(query_value(query, "suppressed").as_deref() == Some("1")) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(500, &error),
            },
            (Method::Post, "/api/speaker") => match set_speaker(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Get, "/api/capture") => match query_value(query, "path") {
                Some(path) => match capture(&path) {
                    Ok(value) => respond(
                        200,
                        "application/json; charset=utf-8",
                        &serde_json::to_vec(&value).unwrap_or_default(),
                    ),
                    Err(error) => api_error(404, &error),
                },
                None => api_error(400, "missing path"),
            },
            (Method::Get, _) => api_error(404, "not found"),
            _ => api_error(405, "method not allowed"),
        };

        let response = OutgoingResponse::new(headers);
        let _ = response.set_status_code(status);
        let Ok(outgoing) = response.body() else {
            return;
        };
        ResponseOutparam::set(response_out, Ok(response));
        let Ok(stream) = outgoing.write() else {
            return;
        };
        for chunk in body.chunks(4096) {
            if stream.blocking_write_and_flush(chunk).is_err() {
                break;
            }
        }
        drop(stream);
        let _ = OutgoingBody::finish(outgoing, None);
    }
}

fn respond(status: u16, content_type: &str, body: &[u8]) -> (Fields, u16, Vec<u8>) {
    let fields = Fields::from_list(&[(
        "content-type".to_string(),
        content_type.as_bytes().to_vec(),
    )])
    .unwrap_or_else(|_| Fields::new());
    let _ = fields.set(
        "cache-control",
        &[b"no-store".to_vec()],
    );
    (fields, status, body.to_vec())
}

fn api_error(status: u16, message: &str) -> (Fields, u16, Vec<u8>) {
    respond(
        status,
        "application/json; charset=utf-8",
        &serde_json::to_vec(&json!({"error": message})).unwrap_or_default(),
    )
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| percent_decode(value))
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
                if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
                out.push(bytes[index]);
                index += 1;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn set_speaker(query: &str) -> Result<Value, String> {
    let path = query_value(query, "path").ok_or("missing path")?;
    let speaker = query_value(query, "speaker").ok_or("missing speaker")?;
    let name = query_value(query, "name").ok_or("missing name")?;
    if !path.starts_with("captures/") || path.contains("..") {
        return Err("invalid capture path".into());
    }
    let name = name.trim();
    if name.is_empty() || name.len() > 80 || speaker.is_empty() {
        return Err("invalid speaker name".into());
    }
    let speakers_path = format!("{path}/speakers.json");
    let mut speakers = read_json(&speakers_path).unwrap_or_else(|_| json!({}));
    if !speakers.is_object() {
        speakers = json!({});
    }
    let person_id = slug(name);
    speakers[&speaker] = json!({
        "name": name,
        "person_id": person_id,
        "source": "manual"
    });
    let bytes = serde_json::to_vec_pretty(&speakers).map_err(|error| error.to_string())?;
    vfs_mut::write(
        &speakers_path,
        &bytes,
        "application/json",
        None,
        None,
    )
    .map_err(|error| format!("write speakers: {error:?}"))?;

    // Manual naming is also an enrollment request. The Scribe enrichment
    // worker owns the audio/model call and turns this into a durable template.
    let queue_path = "voiceprints/learn.json";
    let mut queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    if !queue.is_array() {
        queue = json!([]);
    }
    if let Some(items) = queue.as_array_mut() {
        let already = items.iter().any(|item| {
            item.get("path").and_then(Value::as_str) == Some(path.as_str())
                && item.get("speaker").and_then(Value::as_str) == Some(speaker.as_str())
        });
        if !already {
            items.push(json!({
                "path": path,
                "speaker": speaker,
                "name": name,
                "person_id": person_id,
            }));
        }
    }
    let queue_bytes = serde_json::to_vec_pretty(&queue).map_err(|error| error.to_string())?;
    vfs_mut::write(queue_path, &queue_bytes, "application/json", None, None)
        .map_err(|error| format!("write voiceprint queue: {error:?}"))?;

    Ok(json!({ "ok": true, "speakers": speakers }))
}

/// Hide empty fragments, single-word recordings, and known one-off captures.
/// The bundle stays in VFS; this is a presentation filter, not deletion.
fn is_low_signal(title: &str, summary: &str, text: &str) -> bool {
    if text.split_whitespace().count() < MIN_CAPTURE_WORDS {
        return true;
    }
    let title = title.trim().to_lowercase();
    if LOW_SIGNAL_TITLES.iter().any(|low| title == *low) {
        return true;
    }
    let summary = summary.to_lowercase();
    [
        "single word",
        "unclear remark",
        "brief, unclear",
        "no supported plans or decisions",
        "fragmentary",
        "the transcript contains the single",
        "the only clear remark",
    ]
    .iter()
    .any(|low| summary.contains(low))
}

fn captures(show_suppressed: bool) -> Result<Value, String> {
    let mut cards = Vec::new();
    for source in dirs("captures")? {
        for date in dirs(&source.path)? {
            for capture in dirs(&date.path)? {
                let summary = read_text(&format!("{}/summary.md", capture.path)).unwrap_or_default();
                let actions = read_json(&format!("{}/actions.json", capture.path))
                    .ok()
                    .and_then(|value| value.as_array().map(|items| items.len()))
                    .unwrap_or(0);
                let meta = read_json(&format!("{}/capture.json", capture.path)).ok();
                let started_at_ms = meta
                    .as_ref()
                    .and_then(|value| value.get("started_at_ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let ended_at_ms = meta
                    .as_ref()
                    .and_then(|value| value.get("ended_at_ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(started_at_ms);
                let transcript = read_json(&format!("{}/transcript.json", capture.path)).ok();
                let text = transcript
                    .as_ref()
                    .and_then(|value| {
                        value
                            .get("text")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| {
                                value.get("segments").and_then(Value::as_array).map(|segments| {
                                    segments
                                        .iter()
                                        .filter_map(|segment| {
                                            segment.get("text").and_then(Value::as_str)
                                        })
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                            })
                    })
                    .unwrap_or_default();
                let title = meta
                    .as_ref()
                    .and_then(|value| value.get("title"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| title(&summary))
                    .unwrap_or_else(|| base(&capture.path));
                let low_signal = is_low_signal(&title, &summary, &text);
                if low_signal != show_suppressed {
                    continue;
                }
                let words = text.split_whitespace().count();
                let speakers = transcript
                    .as_ref()
                    .map(speaker_count)
                    .unwrap_or(0);
                let duration_ms = ended_at_ms.saturating_sub(started_at_ms);
                let suppressed_until_ms = ended_at_ms.saturating_add(7 * 24 * 60 * 60 * 1000);
                cards.push(json!({
                    "path": capture.path,
                    "source": base(&source.path),
                    "date": base(&date.path),
                    "id": base(&capture.path),
                    "title": title,
                    "summary": summary,
                    "preview": text.chars().take(220).collect::<String>(),
                    "actions": actions,
                    "speakers": speakers,
                    "words": words,
                    "duration_ms": duration_ms,
                    "suppressed": low_signal,
                    "suppressed_until_ms": suppressed_until_ms,
                    "started_at_ms": started_at_ms,
                    "ended_at_ms": ended_at_ms,
                }));
            }
        }
    }
    cards.sort_by(|a, b| {
        b.get("started_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .cmp(&a.get("started_at_ms").and_then(Value::as_u64).unwrap_or(0))
            .then_with(|| {
                b.get("path")
                    .and_then(Value::as_str)
                    .cmp(&a.get("path").and_then(Value::as_str))
            })
    });
    Ok(json!({"captures": cards}))
}

fn speaker_count(transcript: &Value) -> usize {
    let mut speakers = std::collections::BTreeSet::new();
    for item in transcript
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            transcript
                .get("words")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
    {
        if item.get("is_user").and_then(Value::as_bool) == Some(true) {
            speakers.insert("You".to_string());
        } else if let Some(speaker) = item.get("speaker").and_then(Value::as_str) {
            speakers.insert(speaker.to_string());
        } else if let Some(speaker) = item.get("speaker").and_then(Value::as_i64) {
            speakers.insert(speaker.to_string());
        }
    }
    speakers.len()
}

fn capture(path: &str) -> Result<Value, String> {
    if !path.starts_with("captures/") || path.contains("..") {
        return Err("invalid capture path".into());
    }
    let summary = read_text(&format!("{path}/summary.md")).unwrap_or_default();
    let actions = read_json(&format!("{path}/actions.json")).unwrap_or_else(|_| json!([]));
    let transcript = read_json(&format!("{path}/transcript.json"))?;
    let meta = read_json(&format!("{path}/capture.json")).unwrap_or_else(|_| json!({}));
    let speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
    let text = transcript
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let started = meta.get("started_at_ms").and_then(Value::as_u64).unwrap_or(0);
    let ended = meta.get("ended_at_ms").and_then(Value::as_u64).unwrap_or(started);
    Ok(json!({
        "path": path,
        "title": meta
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| title(&summary))
            .unwrap_or_else(|| base(path)),
        "summary": summary,
        "actions": actions,
        "transcript": transcript,
        "speakers": speakers,
        "words": text.split_whitespace().count(),
        "speaker_count": speaker_count(&transcript),
        "duration_ms": ended.saturating_sub(started),
        "started_at_ms": meta.get("started_at_ms").cloned().unwrap_or(Value::Null),
        "ended_at_ms": meta.get("ended_at_ms").cloned().unwrap_or(Value::Null),
    }))
}

fn dirs(parent: &str) -> Result<Vec<vfs::Entry>, String> {
    let mut entries = Vec::new();
    let mut offset = 0;
    loop {
        let page = vfs::ls(parent, offset, 256).map_err(|error| format!("{error:?}"))?;
        entries.extend(
            page.entries
                .into_iter()
                .filter(|entry| entry.kind == vfs::Kind::Directory),
        );
        match page.next {
            Some(next) => offset = next,
            None => break,
        }
    }
    Ok(entries)
}

fn read_text(path: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut offset = 0;
    loop {
        let chunk = vfs::read(path, offset, MAX_READ).map_err(|error| format!("{error:?}"))?;
        let len = chunk.data.len();
        bytes.extend(chunk.data);
        if chunk.eof || len == 0 {
            break;
        }
        offset += len as u64;
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn read_json(path: &str) -> Result<Value, String> {
    serde_json::from_str(&read_text(path)?).map_err(|error| error.to_string())
}

fn base(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn title(summary: &str) -> Option<String> {
    summary
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| {
            let line = line
                .trim_start_matches('#')
                .trim()
                .trim_start_matches("Overview:")
                .trim();
            let first = line.split(". ").next().unwrap_or(line).trim_end_matches('.');
            first.chars().take(90).collect()
        })
        .filter(|line: &String| !line.is_empty())
}
