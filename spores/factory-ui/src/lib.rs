//! Factory UI: the loop's transcript, read out of the VFS that wrote it.
//!
//! Every byte this page shows comes from `soil:vfs`, and every comment it takes
//! goes back through `soil:vfs-mut` as one immutable object. There is no index
//! and no session state in the guest, so the page *is* the transcript: mount the
//! same root with any other tool and the same facts are there.
//!
//! The interesting part is the comment anchor. A comment records the content id
//! of the segment it was written against, the byte range inside that file, and
//! the quoted text. A rename cannot move it (the content id did not change), and
//! a rewrite cannot silently re-point it: the reader compares content ids and
//! says "written against an earlier revision" rather than attaching the comment
//! to whatever now occupies those bytes.

wit_bindgen::generate!({
    path: "wit",
    world: "factory-ui",
    generate_all,
});

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasi::http::types::{
    Fields, IncomingBody, IncomingRequest, Method, OutgoingBody, OutgoingResponse, ResponseOutparam,
};

use soil::vfs::vfs;
use soil::vfs_mut::vfs_mut;

const STYLE: &str = include_str!("style.css");
const PREFIX: &str = "transcripts";
const MAX_READ: u32 = 1024 * 1024;
const PAGE: u32 = 512;
/// Who a comment is attributed to. The host owns identity (an SSO ingress names
/// the signed-in person); until this page is behind one, it says so rather than
/// inventing a name.
const AUTHOR: &str = "avery";

struct Component;

export!(Component);

// ---------------------------------------------------------------------------
// The transcript as the writer stored it
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct Head {
    #[serde(default)]
    session: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    head_ref: Option<String>,
    #[serde(default)]
    base_ref: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    segments: Vec<Segment>,
    #[serde(default)]
    tail: Vec<Value>,
    #[serde(default)]
    updated_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct Segment {
    path: String,
    content_id: String,
    #[serde(default)]
    events: u64,
    #[serde(default)]
    first_seq: u64,
    #[serde(default)]
    last_seq: u64,
}

impl Head {
    fn segment_for(&self, seq: u64) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|segment| segment.first_seq <= seq && seq <= segment.last_seq)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Anchor {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    start: u64,
    #[serde(default)]
    end: u64,
    #[serde(default)]
    quote: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Comment {
    schema: String,
    version: u32,
    id: String,
    session: String,
    anchor: Anchor,
    body: String,
    author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    created_at_ms: u64,
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

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

        let (fields, status, body, location) = match (&method, path) {
            (Method::Get, "/") | (Method::Get, "/index.html") => {
                let (status, body) = match board() {
                    Ok(html) => (200, html.into_bytes()),
                    Err(error) => (500, error_page(&error).into_bytes()),
                };
                (respond_headers(status, "text/html; charset=utf-8"), status, body, None)
            }
            (Method::Get, "/healthz") => (
                respond_headers(200, "application/json"),
                200,
                br#"{"ok":true}"#.to_vec(),
                None,
            ),
            (Method::Get, "/api/comment") => {
                let Some(session) = query_value(query, "session") else {
                    return write_response(
                        response_out,
                        respond_headers(400, "application/json"),
                        400,
                        br#"{"error":"missing session"}"#.to_vec(),
                        None,
                    );
                };
                match comments(&session) {
                    Ok(comments) => (
                        respond_headers(200, "application/json"),
                        200,
                        serde_json::to_vec(&comments).unwrap_or_default(),
                        None,
                    ),
                    Err(error) => (
                        respond_headers(500, "application/json"),
                        500,
                        serde_json::to_vec(&serde_json::json!({"error": error}))
                            .unwrap_or_default(),
                        None,
                    ),
                }
            }
            (Method::Post, path) if path.starts_with("/s/") && path.ends_with("/comment") => {
                let session = path
                    .trim_start_matches("/s/")
                    .trim_end_matches("/comment")
                    .trim_end_matches('/')
                    .to_string();
                let body = read_body(&request);
                match write_comment(&session, &parse_form(&body)) {
                    Ok(id) => (
                        respond_headers(303, "text/plain; charset=utf-8"),
                        303,
                        Vec::new(),
                        Some(format!("/s/{session}#{id}")),
                    ),
                    Err(error) => (
                        respond_headers(400, "text/html; charset=utf-8"),
                        400,
                        error_page(&error).into_bytes(),
                        None,
                    ),
                }
            }
            (Method::Get, path) if path == "/s" => (
                respond_headers(405, "text/plain; charset=utf-8"),
                405,
                b"GET /s/<session>".to_vec(),
                None,
            ),
            (Method::Get, path) if path.starts_with("/s/") => {
                let session = path.trim_start_matches("/s/").trim_end_matches('/');
                let (status, body) = match session_page(session) {
                    Ok(html) => (200, html.into_bytes()),
                    Err(error) => (404, error_page(&error).into_bytes()),
                };
                (respond_headers(status, "text/html; charset=utf-8"), status, body, None)
            }
            (Method::Get, _) => (
                respond_headers(404, "text/html; charset=utf-8"),
                404,
                error_page("no such page").into_bytes(),
                None,
            ),
            _ => (
                respond_headers(405, "text/plain; charset=utf-8"),
                405,
                b"method not allowed".to_vec(),
                None,
            ),
        };

        write_response(response_out, fields, status, body, location);
    }
}

fn respond_headers(status: u16, content_type: &str) -> Fields {
    let fields = Fields::from_list(&[(
        "content-type".to_string(),
        content_type.as_bytes().to_vec(),
    )])
    .unwrap_or_else(|_| Fields::new());
    let _ = fields.set("cache-control", &[b"no-store".to_vec()]);
    let _ = status;
    fields
}

fn write_response(
    response_out: ResponseOutparam,
    fields: Fields,
    status: u16,
    body: Vec<u8>,
    location: Option<String>,
) {
    if let Some(location) = location {
        let _ = fields.set("location", &[location.into_bytes()]);
    }
    let response = OutgoingResponse::new(fields);
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

fn read_body(request: &IncomingRequest) -> String {
    let Ok(body) = request.consume() else {
        return String::new();
    };
    let Ok(stream) = body.stream() else {
        return String::new();
    };
    let mut bytes = Vec::new();
    loop {
        match stream.blocking_read(4096) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => {
                bytes.extend_from_slice(&chunk);
                if bytes.len() > 256 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    drop(stream);
    let _ = IncomingBody::finish(body);
    String::from_utf8_lossy(&bytes).to_string()
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn sessions() -> Result<Vec<Head>, String> {
    let mut heads = Vec::new();
    let mut offset = 0u64;
    loop {
        let page = vfs::ls(PREFIX, offset, PAGE).map_err(debug)?;
        for entry in page.entries {
            if entry.kind != vfs::Kind::Directory {
                continue;
            }
            let name = entry.name;
            if name.is_empty() {
                continue;
            }
            if let Ok(mut head) = head(&name) {
                if head.session.is_empty() {
                    head.session = name;
                }
                heads.push(head);
            }
        }
        match page.next {
            Some(next) => offset = next,
            None => break,
        }
    }
    heads.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
    Ok(heads)
}

fn head(session: &str) -> Result<Head, String> {
    let path = format!("{PREFIX}/{session}/head.json");
    let mut head: Head = serde_json::from_slice(&read_bytes(&path)?)
        .map_err(|error| format!("{path}: {error}"))?;
    if head.session.is_empty() {
        head.session = session.to_string();
    }
    Ok(head)
}

fn comments(session: &str) -> Result<Vec<Comment>, String> {
    let dir = format!("{PREFIX}/{session}/comments");
    let mut found = Vec::new();
    let mut offset = 0u64;
    loop {
        let page = match vfs::ls(&dir, offset, PAGE) {
            Ok(page) => page,
            // A session nobody has commented on has no comments directory. That
            // is an empty list, not an error the reader has to handle.
            Err(_) => break,
        };
        for entry in page.entries {
            if entry.kind != vfs::Kind::File || !entry.name.ends_with(".json") {
                continue;
            }
            if let Ok(bytes) = read_bytes(&entry.path) {
                if let Ok(comment) = serde_json::from_slice::<Comment>(&bytes) {
                    found.push(comment);
                }
            }
        }
        match page.next {
            Some(next) => offset = next,
            None => break,
        }
    }
    found.sort_by_key(|comment| comment.created_at_ms);
    Ok(found)
}

/// Every event in the transcript, sealed first, each with the segment it came
/// from (or `None` for the live tail).
fn events(head: &Head) -> Result<Vec<(Value, Option<Segment>)>, String> {
    let mut out = Vec::new();
    for segment in &head.segments {
        let text = read_text(&segment.path)?;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                out.push((value, Some(segment.clone())));
            }
        }
    }
    for value in &head.tail {
        out.push((value.clone(), None));
    }
    Ok(out)
}

fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut offset = 0u64;
    loop {
        let chunk = vfs::read(path, offset, MAX_READ).map_err(debug)?;
        let len = chunk.data.len();
        bytes.extend(chunk.data);
        if chunk.eof || len == 0 {
            break;
        }
        offset += len as u64;
    }
    Ok(bytes)
}

fn read_text(path: &str) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&read_bytes(path)?).to_string())
}

fn debug<E: std::fmt::Debug>(error: E) -> String {
    format!("{error:?}")
}

// ---------------------------------------------------------------------------
// Writing a comment
// ---------------------------------------------------------------------------

fn write_comment(session: &str, form: &HashMap<String, String>) -> Result<String, String> {
    let body = form.get("body").map(|text| text.trim()).unwrap_or("");
    if body.is_empty() {
        return Err("a comment needs a body".to_string());
    }
    let head = head(session)?;
    let seq = form
        .get("seq")
        .and_then(|seq| seq.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let quote = form.get("quote").cloned().unwrap_or_default();
    let anchor = locate(&head, seq, &quote)?;
    let created_at_ms = now_ms();
    let id = comment_id(session, &anchor, body, created_at_ms);
    let comment = Comment {
        schema: "fungos.transcript/comment@1".to_string(),
        version: 1,
        id: id.clone(),
        session: session.to_string(),
        anchor,
        body: body.to_string(),
        author: AUTHOR.to_string(),
        parent: form
            .get("parent")
            .filter(|parent| !parent.trim().is_empty())
            .cloned(),
        created_at_ms,
    };
    let path = format!("{PREFIX}/{session}/comments/{id}.json");
    let bytes = serde_json::to_vec_pretty(&comment).map_err(debug)?;
    let _ = vfs_mut::mkdir(&format!("{PREFIX}/{session}/comments"));
    vfs_mut::write(&path, &bytes, "application/json", None, None).map_err(debug)?;
    Ok(id)
}

/// Turn "this event and this quote" into "these bytes of this file".
///
/// The client is not trusted for offsets: it names the event and the text it
/// selected, and the reader finds that text in the bytes it is about to anchor
/// to. A quote that is not there is refused, because a comment that was never
/// anchored to anything is worse than a comment that failed.
fn locate(head: &Head, seq: u64, quote: &str) -> Result<Anchor, String> {
    if seq == 0 && quote.trim().is_empty() {
        return Ok(Anchor {
            path: format!("{PREFIX}/{}/head.json", head.session),
            content_id: None,
            seq: 0,
            start: 0,
            end: 0,
            quote: String::new(),
        });
    }
    let needle = quote.trim();
    if needle.is_empty() {
        return Err("select some text in the transcript first".to_string());
    }

    // Locate the event's own line, and where that line starts in the file. An
    // identical phrase elsewhere in the segment must not capture the comment.
    let Some(segment) = head.segment_for(seq) else {
        // The live tail. Its bytes are the event's canonical JSON line, which is
        // exactly what the writer joins into the next segment, so the offsets stay
        // meaningful - but relative to that line, not to a file that does not
        // exist yet. No content id is claimed, so the reader says "live tail".
        let event = head
            .tail
            .iter()
            .find(|event| event.get("seq").and_then(Value::as_u64) == Some(seq))
            .ok_or_else(|| format!("no event {seq} in this transcript"))?;
        let line = serde_json::to_vec(event).map_err(debug)?;
        let start = find(&line, needle.as_bytes())
            .ok_or_else(|| "that text is not in the event any more".to_string())?;
        return Ok(Anchor {
            path: String::new(),
            content_id: None,
            seq,
            start: start as u64,
            end: (start + needle.len()) as u64,
            quote: needle.to_string(),
        });
    };

    let bytes = read_bytes(&segment.path)?;
    let Some((line, line_start)) = event_line(&bytes, seq) else {
        return Err(format!("event {seq} is not in {}", segment.path));
    };
    let within = find(line, needle.as_bytes())
        .ok_or_else(|| "that text is not in the event any more".to_string())?;
    let start = line_start + within;
    Ok(Anchor {
        path: segment.path.clone(),
        content_id: Some(segment.content_id.clone()),
        seq,
        start: start as u64,
        end: (start + needle.len()) as u64,
        quote: needle.to_string(),
    })
}

/// The bytes of the line holding `seq`, and that line's offset in the file.
fn event_line(bytes: &[u8], seq: u64) -> Option<(&[u8], usize)> {
    let mut offset = 0usize;
    for line in bytes.split(|byte| *byte == b'\n') {
        let matches = serde_json::from_slice::<Value>(line)
            .ok()
            .and_then(|value| value.get("seq").and_then(Value::as_u64))
            == Some(seq);
        if matches {
            return Some((line, offset));
        }
        offset += line.len() + 1;
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn comment_id(session: &str, anchor: &Anchor, body: &str, created_at_ms: u64) -> String {
    // Content-derived so the same comment written twice is one object, with the
    // timestamp included because two people may say the same thing.
    let material = format!(
        "{session}\u{1f}{}\u{1f}{}\u{1f}{body}\u{1f}{created_at_ms}",
        anchor.path, anchor.seq
    );
    format!("c-{:016x}", fnv1a(material.as_bytes()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn now_ms() -> u64 {
    // `wasi:clocks` is granted for the HTTP types, so the wall clock is available
    // to the guest through std on this target.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Presentation
// ---------------------------------------------------------------------------

fn board() -> Result<String, String> {
    let sessions = sessions()?;
    let mut list = String::new();
    if sessions.is_empty() {
        list.push_str(r#"<p class="empty">No transcripts in this root yet. A session appears here once a control plane writes its first segment.</p>"#);
    }
    for head in &sessions {
        let events = head.segments.iter().map(|s| s.events).sum::<u64>() + head.tail.len() as u64;
        list.push_str(&format!(
            r#"<a class="session" href="/s/{id}">
  <div class="t">{title}</div>
  <div class="s"><span class="pill {status}">{status}</span> {events} events · {segments} segments · {when}{ref_line}</div>
</a>"#,
            id = esc(&head.session),
            title = esc(head.title.as_deref().unwrap_or(&head.session)),
            status = esc(&head.status),
            events = events,
            segments = head.segments.len(),
            when = esc(&ago(head.updated_at_ms)),
            ref_line = match &head.head_ref {
                Some(reference) => format!(" · <code>{}</code>", esc(reference)),
                None => String::new(),
            },
        ));
    }
    Ok(page(
        "Factory",
        &format!(
            r#"<h1>The factory</h1>
<p class="meta">Transcripts as they were written: a head ref per session, sealed segment objects, and comments anchored to bytes.</p>
<div class="sessions">{list}</div>
<p class="foot">{count} session(s) in this root. Every line above came from <code>soil:vfs</code>; nothing is cached.</p>"#,
            count = sessions.len()
        ),
    ))
}

fn session_page(session: &str) -> Result<String, String> {
    let head = head(session)?;
    let comments = comments(session).unwrap_or_default();
    let by_seq: HashMap<u64, Vec<&Comment>> = {
        let mut map: HashMap<u64, Vec<&Comment>> = HashMap::new();
        for comment in &comments {
            map.entry(comment.anchor.seq).or_default().push(comment);
        }
        map
    };
    let current: HashMap<&str, &str> = head
        .segments
        .iter()
        .map(|segment| (segment.path.as_str(), segment.content_id.as_str()))
        .collect();

    // A streamed answer arrives as deltas. The transcript should read as the
    // answer, not as the wire: consecutive deltas from one run become one block,
    // anchored to the first delta's event so a comment still has one seq.
    let mut rows: Vec<(Value, Option<Segment>, String)> = Vec::new();
    let mut streamed: Option<(Value, Option<Segment>, String)> = None;
    for (event, segment) in events(&head)? {
        let kind = event.get("event").and_then(Value::as_str).unwrap_or("event");
        if kind == "delta" {
            let text = event
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match streamed.as_mut() {
                Some((_, _, buffered)) => buffered.push_str(&text),
                None => streamed = Some((event, segment, text)),
            }
            continue;
        }
        if let Some(block) = streamed.take() {
            rows.push(block);
        }
        let text = event
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                event
                    .pointer("/activity/label")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        rows.push((event, segment, text));
    }
    if let Some(block) = streamed.take() {
        rows.push(block);
    }

    let mut body = String::new();
    for (event, segment, text) in rows {
        let seq = event.get("seq").and_then(Value::as_u64).unwrap_or(0);
        let kind = event.get("event").and_then(Value::as_str).unwrap_or("event");
        let role = event.get("role").and_then(Value::as_str).unwrap_or("");
        let status = event
            .pointer("/activity/status")
            .and_then(Value::as_str)
            .unwrap_or("");
        let (path, content_id) = match &segment {
            Some(segment) => (segment.path.clone(), segment.content_id.clone()),
            None => (format!("{PREFIX}/{session}/head.json"), String::new()),
        };
        body.push_str(&format!(
            r#"<div class="ev {kind}" id="e{seq}" data-seq="{seq}" data-path="{path}" data-cid="{cid}">
  <div class="who"><span class="seq">#{seq}</span>{who}{status}</div>
  <div class="text">{text}</div>
</div>"#,
            kind = esc(kind),
            seq = seq,
            path = esc(&path),
            cid = esc(&content_id),
            who = if role.is_empty() {
                String::new()
            } else {
                esc(role)
            },
            status = if status.is_empty() {
                String::new()
            } else {
                format!(r#" <span class="pill">{}</span>"#, esc(status))
            },
            text = esc(&text),
        ));
        if let Some(thread) = by_seq.get(&seq) {
            for comment in thread {
                body.push_str(&render_comment(comment, &current));
            }
        }
    }
    let unattached = by_seq.get(&0).cloned().unwrap_or_default();
    let mut top = String::new();
    for comment in unattached {
        top.push_str(&render_comment(comment, &current));
    }

    let head_ref = head
        .head_ref
        .as_deref()
        .map(|reference| format!(r#" · <code>{}</code>"#, esc(reference)))
        .unwrap_or_default();
    let model = head
        .model
        .as_deref()
        .map(|model| format!(" · {}", esc(model)))
        .unwrap_or_default();
    let repo = head
        .repo
        .as_deref()
        .map(|repo| format!(" · {}", esc(repo)))
        .unwrap_or_default();

    Ok(page(
        head.title.as_deref().unwrap_or(session),
        &format!(
            r#"<p class="meta"><a href="/">← all sessions</a></p>
<h1>{title}</h1>
<p class="meta"><span class="pill {status}">{status}</span> {segments} sealed segment(s) · {events} events{head_ref}{repo}{model} · updated {when}</p>
{top}
<h2>Transcript</h2>
<div id="transcript">{body}</div>
<h2>Comment on this transcript</h2>
<form class="composer" method="post" action="/s/{session}/comment">
  <textarea id="body" name="body" required placeholder="Select text above to attach this comment to it, or leave the selection empty to comment on the session."></textarea>
  <div class="quote" id="quote-preview" hidden></div>
  <input type="hidden" id="seq" name="seq" value="0">
  <input type="hidden" id="quote" name="quote" value="">
  <div class="row">
    <span class="meta">anchored to bytes · written by <code>{author}</code></span>
    <button class="primary" type="submit">Comment</button>
  </div>
</form>
<h2>Comments ({count})</h2>
<div class="comments">{all}</div>
<p class="foot">Segment content ids are the anchors. A comment whose segment has changed says so rather than moving.</p>
<script>{script}</script>"#,
            title = esc(head.title.as_deref().unwrap_or(session)),
            status = esc(&head.status),
            segments = head.segments.len(),
            events = head.segments.iter().map(|s| s.events).sum::<u64>() + head.tail.len() as u64,
            head_ref = head_ref,
            repo = repo,
            model = model,
            when = esc(&ago(head.updated_at_ms)),
            session = esc(session),
            body = body,
            top = if top.is_empty() {
                String::new()
            } else {
                format!(r#"<h2>On the session</h2><div class="comments">{top}</div>"#)
            },
            author = AUTHOR,
            count = comments.len(),
            all = if comments.is_empty() {
                r#"<p class="empty">No comments yet.</p>"#.to_string()
            } else {
                comments
                    .iter()
                    .map(|comment| render_comment(comment, &current))
                    .collect()
            },
            script = SCRIPT,
        ),
    ))
}

fn render_comment(comment: &Comment, current: &HashMap<&str, &str>) -> String {
    let (state, badge) = match (comment.anchor.content_id.as_deref(), current.get(comment.anchor.path.as_str())) {
        (None, _) => ("", String::new()),
        (Some(_), None) => (
            " older",
            r#"<span class="badge missing">the file is gone</span>"#.to_string(),
        ),
        (Some(written), Some(now)) if written == *now => ("", String::new()),
        (Some(_), Some(_)) => (
            " older",
            r#"<span class="badge">written against an earlier revision</span>"#.to_string(),
        ),
    };
    let quote = if comment.anchor.quote.trim().is_empty() {
        String::new()
    } else if comment.anchor.content_id.is_none() {
        format!(
            r#"<div class="anchor">live tail, not sealed yet
<div>{quote}</div></div>"#,
            quote = esc(&comment.anchor.quote),
        )
    } else {
        format!(
            r#"<div class="anchor"><code>{path}</code> bytes {start}–{end}
<div>{quote}</div></div>"#,
            path = esc(&comment.anchor.path),
            start = comment.anchor.start,
            end = comment.anchor.end,
            quote = esc(&comment.anchor.quote),
        )
    };
    format!(
        r#"<div class="comment{state}" id="{id}">
  <div class="head"><code>{author}</code> · {when} {badge}</div>
  <div class="body">{body}</div>
  {quote}
</div>"#,
        state = state,
        id = esc(&comment.id),
        author = esc(&comment.author),
        when = esc(&ago(comment.created_at_ms)),
        badge = badge,
        body = esc(&comment.body),
        quote = quote,
    )
}

fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>{style}</style>
</head>
<body>
<header>
  <a class="brand" href="/">Factory<b>.</b></a>
  <span class="tag">transcripts</span>
  <div class="spacer"></div>
</header>
<main>{body}</main>
</body>
</html>"#,
        title = esc(title),
        style = STYLE,
        body = body,
    )
}

fn error_page(message: &str) -> String {
    page(
        "Factory",
        &format!(
            r#"<p class="meta"><a href="/">← all sessions</a></p>
<h1>That page is not here</h1>
<div class="error">{}</div>"#,
            esc(message)
        ),
    )
}

fn esc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn ago(at_ms: u64) -> String {
    if at_ms == 0 {
        return "unknown".to_string();
    }
    let now = now_ms();
    if now <= at_ms {
        return "just now".to_string();
    }
    let seconds = (now - at_ms) / 1000;
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

fn parse_form(body: &str) -> HashMap<String, String> {
    let mut form = HashMap::new();
    for pair in body.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key.replace('+', " ").as_str());
        let value = percent_decode(value.replace('+', " ").as_str());
        form.insert(key, value);
    }
    form
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
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Selecting text in the transcript fills the anchor. The reader never sends
/// offsets: it names the event and quotes the text, and the guest finds those
/// bytes in the file it is about to anchor to.
const SCRIPT: &str = r#"
const seqInput = document.getElementById('seq');
const quoteInput = document.getElementById('quote');
const preview = document.getElementById('quote-preview');
const body = document.getElementById('body');
document.addEventListener('mouseup', () => {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return;
  const range = selection.getRangeAt(0);
  let node = range.startContainer;
  if (node.nodeType === 3) node = node.parentElement;
  const event = node && node.closest ? node.closest('.ev') : null;
  if (!event) return;
  const text = event.querySelector('.text');
  if (!text || !text.contains(range.startContainer)) return;
  const quote = selection.toString();
  if (!quote.trim()) return;
  document.querySelectorAll('.ev.selected').forEach(el => el.classList.remove('selected'));
  event.classList.add('selected');
  seqInput.value = event.dataset.seq;
  quoteInput.value = quote;
  preview.textContent = '#' + event.dataset.seq + ' · ' + quote.trim().slice(0, 400);
  preview.hidden = false;
  body.placeholder = 'Comment on the selected text…';
});
document.querySelector('form.composer').addEventListener('submit', () => {
  if (!quoteInput.value.trim()) seqInput.value = '0';
});
"#;
