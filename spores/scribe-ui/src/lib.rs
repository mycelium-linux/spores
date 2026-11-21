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

use soil::index::index;
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
        let target = request.path_with_query().unwrap_or_else(|| "/".to_string());
        let (path, query) = target
            .split_once('?')
            .map(|(path, query)| (path, query))
            .unwrap_or((target.as_str(), ""));

        let (headers, status, body) = match (method, path) {
            (Method::Get, "/") | (Method::Get, "/index.html") => {
                respond(200, "text/html; charset=utf-8", HTML.as_bytes())
            }
            (Method::Get, path) if path == "/c" || path.starts_with("/c/") => {
                respond(200, "text/html; charset=utf-8", HTML.as_bytes())
            }
            (Method::Get, "/api/captures") => {
                let offset = query_value(query, "offset")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let limit = query_value(query, "limit")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(50)
                    .clamp(1, 100);
                let from = query_value(query, "from").filter(|value| !value.is_empty());
                let to = query_value(query, "to").filter(|value| !value.is_empty());
                match captures(
                    query_value(query, "suppressed").as_deref() == Some("1"),
                    offset,
                    limit,
                    from.as_deref(),
                    to.as_deref(),
                ) {
                    Ok(value) => respond(
                        200,
                        "application/json; charset=utf-8",
                        &serde_json::to_vec(&value).unwrap_or_default(),
                    ),
                    Err(error) => api_error(500, &error),
                }
            }
            (Method::Get, "/api/search") => {
                let from = query_value(query, "from").filter(|value| !value.is_empty());
                let to = query_value(query, "to").filter(|value| !value.is_empty());
                match query_value(query, "q").filter(|value| !value.trim().is_empty()) {
                    Some(text) => match search_captures(&text, from.as_deref(), to.as_deref()) {
                        Ok(value) => respond(
                            200,
                            "application/json; charset=utf-8",
                            &serde_json::to_vec(&value).unwrap_or_default(),
                        ),
                        Err(error) => api_error(500, &error),
                    },
                    None => api_error(400, "missing q"),
                }
            }
            (Method::Post, "/api/speaker") => match set_speaker(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Post, "/api/swap") => match swap_speakers(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Post, "/api/relearn") => match relearn(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Post, "/api/restyle") => match queue_restyle(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Post, "/api/voicefix") => match queue_voicefix(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Post, "/api/merge") => match queue_merge(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Post, "/api/clean") => match queue_clean(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Get, "/api/vocabulary") => match read_text("scribe/vocabulary.txt") {
                Ok(text) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&json!({"ok": true, "text": text})).unwrap_or_default(),
                ),
                Err(_) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&json!({"ok": true, "text": ""})).unwrap_or_default(),
                ),
            },
            (Method::Post, "/api/vocabulary/save") => match save_vocabulary(query) {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(400, &error),
            },
            (Method::Get, "/api/profile") => match read_json("profile.json") {
                Ok(profile) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&json!({"ok": true, "profile": profile}))
                        .unwrap_or_default(),
                ),
                Err(_) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&json!({"ok": false})).unwrap_or_default(),
                ),
            },
            (Method::Get, "/api/capture") => match query_value(query, "path")
                .or_else(|| query_value(query, "id").and_then(|id| find_capture(&id).ok()))
            {
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
    let fields =
        Fields::from_list(&[("content-type".to_string(), content_type.as_bytes().to_vec())])
            .unwrap_or_else(|_| Fields::new());
    let _ = fields.set("cache-control", &[b"no-store".to_vec()]);
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

fn query_values(query: &str, key: &str) -> Vec<String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == key).then(|| percent_decode(value))
        })
        .collect()
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

fn canonical_speaker(raw: &str) -> String {
    if !raw.is_empty() && raw.chars().all(|ch| ch.is_ascii_digit()) {
        if let Ok(value) = raw.parse::<u32>() {
            return format!("SPEAKER_{value:02}");
        }
    }
    raw.to_string()
}

fn valid_capture(path: &str) -> bool {
    path.starts_with("captures/") && !path.contains("..")
}

fn write_json(path: &str, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    vfs_mut::write(path, &bytes, "application/json", None, None)
        .map(|_| ())
        .map_err(|error| format!("write {path}: {error:?}"))
}

fn read_overrides(path: &str) -> Value {
    let value = read_json(&format!("{path}/speaker_overrides.json")).unwrap_or_else(|_| json!([]));
    if value.is_array() {
        value
    } else {
        json!([])
    }
}

fn queue_learning(
    path: &str,
    speaker: &str,
    name: &str,
    person_id: &str,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(), String> {
    let queue_path = "voiceprints/learn.json";
    let mut queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    if !queue.is_array() {
        queue = json!([]);
    }
    if let Some(items) = queue.as_array_mut() {
        let already = items.iter().any(|item| {
            item.get("path").and_then(Value::as_str) == Some(path)
                && item.get("speaker").and_then(Value::as_str) == Some(speaker)
                && item.get("start").and_then(Value::as_f64) == start
        });
        if !already {
            let mut item = json!({
                "path": path,
                "speaker": speaker,
                "name": name,
                "person_id": person_id,
                "force": true,
            });
            if let (Some(start), Some(end)) = (start, end) {
                item["start"] = json!(start);
                item["end"] = json!(end);
            }
            items.push(item);
        }
    }
    write_json(queue_path, &queue)
}

fn queue_merge(query: &str) -> Result<Value, String> {
    let mut paths = query_values(query, "paths");
    paths.retain(|path| !path.trim().is_empty());
    paths.sort();
    paths.dedup();
    if paths.len() < 2 {
        return Err("select at least two captures to merge".into());
    }
    if paths.iter().any(|path| !valid_capture(path)) {
        return Err("invalid capture path in merge".into());
    }
    let queue_path = "scribe/merge.json";
    let mut queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    if !queue.is_array() {
        queue = json!([]);
    }
    if let Some(items) = queue.as_array_mut() {
        let already = items.iter().any(|item| {
            item.get("paths")
                .and_then(Value::as_array)
                .is_some_and(|existing| {
                    let mut existing: Vec<&str> =
                        existing.iter().filter_map(Value::as_str).collect();
                    existing.sort();
                    existing == paths.iter().map(String::as_str).collect::<Vec<_>>()
                })
        });
        if !already {
            items.push(json!({"paths": paths}));
        }
    }
    write_json(queue_path, &queue)?;
    Ok(json!({"ok": true, "queued": true, "count": paths.len()}))
}

fn set_speaker(query: &str) -> Result<Value, String> {
    let path = query_value(query, "path").ok_or("missing path")?;
    let speaker = query_value(query, "speaker")
        .map(|value| canonical_speaker(&value))
        .ok_or("missing speaker")?;
    let name = query_value(query, "name").ok_or("missing name")?;
    if !valid_capture(&path) {
        return Err("invalid capture path".into());
    }
    let name = name.trim();
    if name.is_empty() || name.len() > 80 || speaker.is_empty() {
        return Err("invalid speaker name".into());
    }
    let person_id = slug(name);
    let scope = query_value(query, "scope").unwrap_or_else(|| "all".to_string());
    if scope == "line" {
        let start = query_value(query, "start")
            .and_then(|value| value.parse::<f64>().ok())
            .ok_or("line scope needs start")?;
        let end = query_value(query, "end")
            .and_then(|value| value.parse::<f64>().ok())
            .ok_or("line scope needs end")?;
        if !(end > start) || !start.is_finite() || !end.is_finite() {
            return Err("invalid line window".into());
        }
        let overrides_path = format!("{path}/speaker_overrides.json");
        let mut overrides = read_overrides(&path);
        if let Some(items) = overrides.as_array_mut() {
            items.retain(|item| {
                let old_start = item.get("start").and_then(Value::as_f64).unwrap_or(-1.0);
                let old_end = item.get("end").and_then(Value::as_f64).unwrap_or(-1.0);
                (old_start - start).abs() > 0.01 || (old_end - end).abs() > 0.01
            });
            items.push(json!({
                "start": start,
                "end": end,
                "speaker": speaker,
                "name": name,
                "person_id": person_id,
                "source": "manual",
            }));
        }
        write_json(&overrides_path, &overrides)?;
        queue_learning(&path, &speaker, name, &person_id, Some(start), Some(end))?;
        let speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
        return Ok(
            json!({"ok": true, "scope": "line", "speakers": speakers, "overrides": overrides}),
        );
    }

    let speakers_path = format!("{path}/speakers.json");
    let mut speakers = read_json(&speakers_path).unwrap_or_else(|_| json!({}));
    if !speakers.is_object() {
        speakers = json!({});
    }
    speakers[&speaker] = json!({
        "name": name,
        "person_id": person_id,
        "source": "manual"
    });
    write_json(&speakers_path, &speakers)?;
    // A bulk rename supersedes earlier one-line corrections for this speaker.
    let mut overrides = read_overrides(&path);
    if let Some(items) = overrides.as_array_mut() {
        items.retain(|item| item.get("speaker").and_then(Value::as_str) != Some(speaker.as_str()));
    }
    write_json(&format!("{path}/speaker_overrides.json"), &overrides)?;
    queue_learning(&path, &speaker, name, &person_id, None, None)?;
    apply_speaker_names(&path, &speakers)?;
    Ok(json!({"ok": true, "scope": "all", "speakers": speakers}))
}

fn swap_text(text: &str, a_name: &str, b_name: &str) -> String {
    const A: &str = "\u{0}scribe-a\u{0}";
    const B: &str = "\u{0}scribe-b\u{0}";
    let text = text.replace(a_name, A).replace(b_name, B);
    text.replace(A, b_name).replace(B, a_name)
}

fn swap_speakers(query: &str) -> Result<Value, String> {
    let path = query_value(query, "path").ok_or("missing path")?;
    let a = query_value(query, "a")
        .map(|value| canonical_speaker(&value))
        .ok_or("missing a")?;
    let b = query_value(query, "b")
        .map(|value| canonical_speaker(&value))
        .ok_or("missing b")?;
    if !valid_capture(&path) || a == b {
        return Err("invalid swap".into());
    }
    let speakers_path = format!("{path}/speakers.json");
    let mut speakers = read_json(&speakers_path).unwrap_or_else(|_| json!({}));
    let Some(map) = speakers.as_object_mut() else {
        return Err("speakers is not an object".into());
    };
    let a_value = map.get(&a).cloned().ok_or("missing speaker a")?;
    let b_value = map.get(&b).cloned().ok_or("missing speaker b")?;
    if a_value.get("name").is_none() || b_value.get("name").is_none() {
        return Err("both speakers need names before swapping".into());
    }
    map.insert(a.clone(), b_value);
    map.insert(b.clone(), a_value);
    write_json(&speakers_path, &speakers)?;

    if let Ok(mut actions) = read_json(&format!("{path}/actions.json")) {
        if let Some(items) = actions.as_array_mut() {
            let a_name = speakers[&a]
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let b_name = speakers[&b]
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("");
            for action in items {
                if let Some(fields) = action.as_object_mut() {
                    for field in ["title", "quote", "owner", "speaker"] {
                        if let Some(text) = fields.get(field).and_then(Value::as_str) {
                            fields
                                .insert(field.to_string(), json!(swap_text(text, a_name, b_name)));
                        }
                    }
                }
            }
            write_json(&format!("{path}/actions.json"), &actions)?;
        }
    }
    apply_speaker_names(&path, &speakers)?;
    Ok(json!({"ok": true, "speakers": speakers}))
}

fn queue_clean(query: &str) -> Result<Value, String> {
    let path = query_value(query, "path").ok_or("missing path")?;
    if !valid_capture(&path) {
        return Err("invalid capture path".into());
    }
    let _ = vfs_mut::mkdir("scribe");
    let queue_path = "scribe/clean.json";
    let mut queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    if !queue.is_array() {
        queue = json!([]);
    }
    if let Some(items) = queue.as_array_mut() {
        if !items
            .iter()
            .any(|item| item.get("path").and_then(Value::as_str) == Some(path.as_str()))
        {
            items.push(json!({"path": path}));
        }
    }
    write_json(queue_path, &queue)?;
    Ok(json!({"ok": true, "queued": path}))
}

fn save_vocabulary(query: &str) -> Result<Value, String> {
    let text = query_value(query, "text").unwrap_or_default();
    if text.len() > 64 * 1024 {
        return Err("vocabulary is too large".into());
    }
    let _ = vfs_mut::mkdir("scribe");
    vfs_mut::write(
        "scribe/vocabulary.txt",
        text.as_bytes(),
        "text/plain",
        None,
        None,
    )
    .map_err(|error| format!("write vocabulary: {error:?}"))?;
    Ok(json!({"ok": true, "bytes": text.len()}))
}

fn queue_restyle(query: &str) -> Result<Value, String> {
    let path = query_value(query, "path").ok_or("missing path")?;
    if !valid_capture(&path) {
        return Err("invalid capture path".into());
    }
    let queue_path = "voiceprints/restyle.json";
    let mut queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    if !queue.is_array() {
        queue = json!([]);
    }
    if let Some(items) = queue.as_array_mut() {
        let already = items
            .iter()
            .any(|item| item.get("path").and_then(Value::as_str) == Some(path.as_str()));
        if !already {
            items.push(json!({"path": path, "style": "design"}));
        }
    }
    write_json(queue_path, &queue)?;
    Ok(json!({"ok": true, "queued": path}))
}

fn queue_voicefix(query: &str) -> Result<Value, String> {
    let path = query_value(query, "path").ok_or("missing path")?;
    if !valid_capture(&path) {
        return Err("invalid capture path".into());
    }
    let queue_path = "voiceprints/voicefix.json";
    let mut queue = read_json(queue_path).unwrap_or_else(|_| json!([]));
    if !queue.is_array() {
        queue = json!([]);
    }
    if let Some(items) = queue.as_array_mut() {
        let already = items
            .iter()
            .any(|item| item.get("path").and_then(Value::as_str) == Some(path.as_str()));
        if !already {
            items.push(json!({"path": path, "source": "scribe-ui"}));
        }
    }
    write_json(queue_path, &queue)?;
    Ok(json!({"ok": true, "queued": path}))
}

fn relearn(query: &str) -> Result<Value, String> {
    let path = query_value(query, "path").ok_or("missing path")?;
    if !valid_capture(&path) {
        return Err("invalid capture path".into());
    }
    let speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
    let mut queued = 0usize;
    if let Some(entries) = speakers.as_object() {
        for (speaker, entry) in entries {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            let person_id = entry
                .get("person_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| slug(name));
            queue_learning(&path, speaker, name, &person_id, None, None)?;
            queued += 1;
        }
    }
    Ok(json!({"ok": true, "queued": queued}))
}

fn apply_speaker_names(path: &str, speakers: &Value) -> Result<(), String> {
    let Some(entries) = speakers.as_object() else {
        return Ok(());
    };
    let labels: Vec<(String, String)> = entries
        .iter()
        .filter_map(|(label, entry)| {
            let name = entry.get("name").and_then(Value::as_str)?.trim();
            (!name.is_empty()).then(|| (label.clone(), name.to_string()))
        })
        .collect();
    if labels.is_empty() {
        return Ok(());
    }
    let actions_path = format!("{path}/actions.json");
    let Ok(mut actions) = read_json(&actions_path) else {
        return Ok(());
    };
    let Some(items) = actions.as_array_mut() else {
        return Ok(());
    };
    let mut changed = false;
    for action in items {
        let Some(fields) = action.as_object_mut() else {
            continue;
        };
        for field in ["title", "quote", "owner", "speaker"] {
            let Some(text) = fields
                .get(field)
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let mut next = text;
            for (label, name) in &labels {
                next = next.replace(label, name);
            }
            if fields.get(field).and_then(Value::as_str) != Some(next.as_str()) {
                fields.insert(field.to_string(), json!(next));
                changed = true;
            }
        }
    }
    if changed {
        write_json(&actions_path, &actions)?;
    }
    Ok(())
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

fn date_in_range(date: &str, from: Option<&str>, to: Option<&str>) -> bool {
    from.is_none_or(|from| date >= from) && to.is_none_or(|to| date <= to)
}

fn capture_card(path: &str) -> Result<Value, String> {
    let summary = read_text(&format!("{path}/summary.md")).unwrap_or_default();
    let actions = read_json(&format!("{path}/actions.json"))
        .ok()
        .and_then(|value| value.as_array().map(|items| items.len()))
        .unwrap_or(0);
    let questions = read_json(&format!("{path}/questions.json"))
        .ok()
        .and_then(|value| value.as_array().map(|items| items.len()))
        .unwrap_or(0);
    let calendar = read_json(&format!("{path}/calendar.json"))
        .ok()
        .and_then(|value| value.as_array().map(|items| items.len()))
        .unwrap_or(0);
    let meta = read_json(&format!("{path}/capture.json")).ok();
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
    let transcript = read_json(&format!("{path}/transcript.json")).ok();
    let text = transcript
        .as_ref()
        .and_then(|value| {
            value
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    value
                        .get("segments")
                        .and_then(Value::as_array)
                        .map(|segments| {
                            segments
                                .iter()
                                .filter_map(|segment| segment.get("text").and_then(Value::as_str))
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
        .unwrap_or_else(|| base(path));
    let low_signal = is_low_signal(&title, &summary, &text);
    let processing = summary.trim().is_empty();
    let date = path.split('/').nth(2).unwrap_or("undated").to_string();
    let source = path.split('/').nth(1).unwrap_or("unknown").to_string();
    let id = path.rsplit('/').next().unwrap_or(path).to_string();
    let words = text.split_whitespace().count();
    let speakers = transcript.as_ref().map(speaker_count).unwrap_or(0);
    let duration_ms = ended_at_ms.saturating_sub(started_at_ms);
    let suppressed_until_ms = ended_at_ms.saturating_add(7 * 24 * 60 * 60 * 1000);
    Ok(json!({
        "path": path,
        "source": source,
        "date": date,
        "id": id,
        "title": title,
        "emoji": meta
            .as_ref()
            .and_then(|value| value.get("emoji"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        "merged_from": meta
            .as_ref()
            .and_then(|value| value.get("merged_from"))
            .cloned()
            .unwrap_or(Value::Null),
        "merged_into": meta
            .as_ref()
            .and_then(|value| value.get("merged_into"))
            .cloned()
            .unwrap_or(Value::Null),
        "summary": summary,
        "preview": text.chars().take(220).collect::<String>(),
        "actions": actions,
        "questions": questions,
        "calendar": calendar,
        "speakers": speakers,
        "words": words,
        "duration_ms": duration_ms,
        "suppressed": low_signal,
        "processing": processing,
        "suppressed_until_ms": suppressed_until_ms,
        "started_at_ms": started_at_ms,
        "ended_at_ms": ended_at_ms,
    }))
}

fn sort_cards(cards: &mut [Value]) {
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
}

fn captures(
    show_suppressed: bool,
    offset: usize,
    limit: usize,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Value, String> {
    let mut cards = Vec::new();
    let mut processing = 0usize;
    let mut processing_latest = 0u64;
    let mut hidden = 0usize;
    for source in dirs("captures")? {
        for date in dirs(&source.path)? {
            let day = base(&date.path);
            if !date_in_range(&day, from, to) {
                continue;
            }
            for capture in dirs(&date.path)? {
                let Ok(card) = capture_card(&capture.path) else {
                    continue;
                };
                if card.get("merged_into").and_then(Value::as_str).is_some() {
                    continue;
                }
                if card
                    .get("processing")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    if card
                        .get("suppressed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        hidden += 1;
                    } else {
                        processing += 1;
                        processing_latest = processing_latest.max(
                            card.get("started_at_ms")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                        );
                    }
                    continue;
                }
                if card
                    .get("suppressed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    != show_suppressed
                {
                    hidden += 1;
                    continue;
                }
                cards.push(card);
            }
        }
    }
    sort_cards(&mut cards);
    let total = cards.len();
    let page: Vec<Value> = cards.into_iter().skip(offset).take(limit).collect();
    let next = offset.saturating_add(page.len()) < total;
    Ok(json!({
        "captures": page,
        "total": total,
        "processing": processing,
        "processing_latest_ms": processing_latest,
        "hidden": hidden,
        "next": next.then_some((offset + page.len()).to_string()),
    }))
}

fn capture_root(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 5 && parts[0] == "captures" {
        Some(parts[..4].join("/"))
    } else {
        None
    }
}

fn search_captures(text: &str, from: Option<&str>, to: Option<&str>) -> Result<Value, String> {
    let query = index::Query {
        text: text.to_string(),
        mode: Some(index::Mode::FullText),
        parent: None,
        media_type_prefix: None,
        offset: 0,
        limit: 256,
    };
    let page = index::search(&query)
        .or_else(|_| {
            index::search(&index::Query {
                mode: Some(index::Mode::Substring),
                ..query.clone()
            })
        })
        .map_err(|error| format!("index search: {error:?}"))?;
    let mut roots = std::collections::BTreeSet::new();
    for hit in page.hits {
        if let Some(root) = capture_root(&hit.path) {
            let date = root.split('/').nth(2).unwrap_or("");
            if date_in_range(date, from, to) {
                roots.insert(root);
            }
        }
    }
    let mut cards = Vec::new();
    for root in roots {
        if let Ok(card) = capture_card(&root) {
            if card.get("merged_into").and_then(Value::as_str).is_none() {
                cards.push(card);
            }
        }
    }
    sort_cards(&mut cards);
    Ok(json!({"captures": cards, "total": cards.len(), "next": null}))
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

fn find_capture(id: &str) -> Result<String, String> {
    if id.is_empty() || id.contains('/') || id.contains("..") {
        return Err("invalid capture id".into());
    }
    for source in dirs("captures")? {
        for date in dirs(&source.path)? {
            for capture in dirs(&date.path)? {
                if base(&capture.path) == id {
                    return Ok(capture.path);
                }
            }
        }
    }
    Err("capture not found".into())
}

fn capture(path: &str) -> Result<Value, String> {
    if !path.starts_with("captures/") || path.contains("..") {
        return Err("invalid capture path".into());
    }
    let summary = read_text(&format!("{path}/summary.md")).unwrap_or_default();
    let actions = read_json(&format!("{path}/actions.json")).unwrap_or_else(|_| json!([]));
    let questions = read_json(&format!("{path}/questions.json")).unwrap_or_else(|_| json!([]));
    let calendar = read_json(&format!("{path}/calendar.json")).unwrap_or_else(|_| json!([]));
    let transcript = read_json(&format!("{path}/transcript.json"))?;
    let meta = read_json(&format!("{path}/capture.json")).unwrap_or_else(|_| json!({}));
    let speakers = read_json(&format!("{path}/speakers.json")).unwrap_or_else(|_| json!({}));
    let overrides = read_overrides(path);
    let design_summary = read_text(&format!("{path}/summary.design.md")).ok();
    let clean_transcript = read_json(&format!("{path}/transcript.clean.json")).ok();
    let text = transcript
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let started = meta
        .get("started_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let ended = meta
        .get("ended_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(started);
    Ok(json!({
        "path": path,
        "id": base(path),
        "title": meta
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| title(&summary))
            .unwrap_or_else(|| base(path)),
        "emoji": meta.get("emoji").and_then(Value::as_str).unwrap_or(""),
        "merged_from": meta.get("merged_from").cloned().unwrap_or(Value::Null),
        "merged_into": meta.get("merged_into").cloned().unwrap_or(Value::Null),
        "summary": summary,
        "design_summary": design_summary,
        "clean_transcript": clean_transcript,
        "actions": actions,
        "questions": questions,
        "calendar": calendar,
        "transcript": transcript,
        "speakers": speakers,
        "overrides": overrides,
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
            let first = line
                .split(". ")
                .next()
                .unwrap_or(line)
                .trim_end_matches('.');
            first.chars().take(40).collect()
        })
        .filter(|line: &String| !line.is_empty())
}
