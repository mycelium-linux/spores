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

const HTML: &str = include_str!("ui.html");
const MAX_READ: u32 = 1024 * 1024;

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
            (Method::Get, "/api/captures") => match captures() {
                Ok(value) => respond(
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_vec(&value).unwrap_or_default(),
                ),
                Err(error) => api_error(500, &error),
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

fn captures() -> Result<Value, String> {
    let mut cards = Vec::new();
    for source in dirs("captures")? {
        for date in dirs(&source.path)? {
            for capture in dirs(&date.path)? {
                let summary = read_text(&format!("{}/summary.md", capture.path)).unwrap_or_default();
                let actions = read_json(&format!("{}/actions.json", capture.path))
                    .ok()
                    .and_then(|value| value.as_array().map(|items| items.len()))
                    .unwrap_or(0);
                let transcript = read_json(&format!("{}/transcript.json", capture.path)).ok();
                let text = transcript
                    .as_ref()
                    .and_then(|value| value.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let speakers = transcript
                    .as_ref()
                    .and_then(|value| value.get("words"))
                    .and_then(Value::as_array)
                    .map(|words| {
                        let mut speakers = words
                            .iter()
                            .filter_map(|word| word.get("speaker").and_then(Value::as_i64))
                            .collect::<Vec<_>>();
                        speakers.sort_unstable();
                        speakers.dedup();
                        speakers.len()
                    })
                    .unwrap_or(0);
                cards.push(json!({
                    "path": capture.path,
                    "source": base(&source.path),
                    "date": base(&date.path),
                    "id": base(&capture.path),
                    "title": title(&summary).unwrap_or_else(|| base(&capture.path)),
                    "summary": summary,
                    "preview": text.chars().take(220).collect::<String>(),
                    "actions": actions,
                    "speakers": speakers,
                }));
            }
        }
    }
    cards.sort_by(|a, b| {
        b.get("path")
            .and_then(Value::as_str)
            .cmp(&a.get("path").and_then(Value::as_str))
    });
    Ok(json!({"captures": cards}))
}

fn capture(path: &str) -> Result<Value, String> {
    if !path.starts_with("captures/") || path.contains("..") {
        return Err("invalid capture path".into());
    }
    let summary = read_text(&format!("{path}/summary.md")).unwrap_or_default();
    let actions = read_json(&format!("{path}/actions.json")).unwrap_or_else(|_| json!([]));
    let transcript = read_json(&format!("{path}/transcript.json"))?;
    Ok(json!({
        "path": path,
        "title": title(&summary).unwrap_or_else(|| base(path)),
        "summary": summary,
        "actions": actions,
        "transcript": transcript,
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
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("Overview:"))
        .map(|line| line.trim_start_matches("Overview:").trim().chars().take(80).collect())
}
