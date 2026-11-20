//! The web lane: answer HTTP with server-rendered pages.
//!
//! The Spore owns the HTML and the projections under it. The host owns routing,
//! the records book, and the clock: `now` is the newest moment the book records,
//! so the same book renders the same page whenever it is read.

wit_bindgen::generate!({
    path: "wit",
    world: "marbles-ui",
    generate_all,
});

use wasi::http::types::{
    Fields, IncomingRequest, Method, OutgoingBody, OutgoingResponse, ResponseOutparam,
};

use soil::records::records;

use crate::model;
use crate::render;

/// The records collection a Marbles store keeps. One collection holds every
/// record type; this view reads `issue/` and `dependency/` and ignores the rest.
const COLLECTION: &str = "marbles";
/// Records per `scan`. A page a few thousand issues wide costs a handful of
/// calls at this size.
/// The host's maximum page. Reading a whole book in two or three calls
/// instead of a dozen is most of the boundary-crossing cost.
const PAGE: u32 = 4096;
/// Enough pages to read a fleet book and not enough to loop forever on a cursor
/// the host keeps handing back.
const MAX_PAGES: usize = 4096;

struct Component;

export!(Component);

impl exports::wasi::http::incoming_handler::Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();
        let target = request.path_with_query().unwrap_or_else(|| "/".to_string());
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));

        if !matches!(method, Method::Get) {
            answer(
                response_out,
                405,
                "text/plain; charset=utf-8",
                b"method not allowed\n",
            );
            return;
        }
        if path == "/healthz" {
            answer(response_out, 200, "text/plain; charset=utf-8", b"ok\n");
            return;
        }

        let shallow = matches!(path, "/board" | "/burndown");
        let started = std::time::Instant::now();
        let loaded = match load(shallow) {
            Ok(loaded) => loaded,
            Err(error) => {
                let page = render::error_page(&model::Book::default(), 503, &error);
                answer(
                    response_out,
                    503,
                    "text/html; charset=utf-8",
                    page.as_bytes(),
                );
                return;
            }
        };
        let book = &loaded.book;

        let project = query_value(query, "project").filter(|value| !value.is_empty());
        let project = project.as_deref();
        let q = query_value(query, "q").unwrap_or_default();
        let days = query_value(query, "days")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(14)
            .clamp(1, 180);
        let stale_days = query_value(query, "stale")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(7)
            .clamp(1, 365);

        let render_started = std::time::Instant::now();
        let (status, body) = match path {
            "/" => (200, render::dashboard(book, project, &q, days, stale_days)),
            "/board" => {
                let limit = query_value(query, "limit")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(50)
                    .clamp(1, 200);
                let page = query_value(query, "page")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let status = query_value(query, "status").filter(|value| !value.is_empty());
                (
                    200,
                    render::board_page(book, project, &q, limit, status.as_deref(), page),
                )
            }
            "/burndown" => (200, render::burndown_page(book, project, &q, days)),
            "/projects" => (200, render::projects_page(book, project, &q, stale_days)),
            "/plans" => (200, render::plans_page(book, project, &q)),
            "/plan" => match query_value(query, "name").filter(|name| !name.is_empty()) {
                Some(name) => (200, render::plan_page(book, &name, project, &q)),
                None => (400, render::error_page(book, 400, "missing plan name")),
            },
            "/issue" => match query_value(query, "id").filter(|id| !id.is_empty()) {
                Some(id) => (200, render::issue_page(book, &id, project, &q)),
                None => (400, render::error_page(book, 400, "missing issue id")),
            },
            _ => (
                404,
                render::error_page(book, 404, &format!("no route for {path}")),
            ),
        };
        let headers = [
            ("x-marbles-entries", loaded.entries.to_string()),
            ("x-marbles-scan-ms", loaded.scan_ms.to_string()),
            ("x-marbles-model-ms", loaded.model_ms.to_string()),
            (
                "x-marbles-render-ms",
                (render_started.elapsed().as_millis() as u64).to_string(),
            ),
            (
                "x-marbles-total-ms",
                (started.elapsed().as_millis() as u64).to_string(),
            ),
        ];
        answer_with(
            response_out,
            status,
            "text/html; charset=utf-8",
            body.as_bytes(),
            &headers,
        );
    }
}

/// Read every live record in the collection, following the cursor to the end.
struct Loaded {
    book: model::Book,
    entries: usize,
    scan_ms: u64,
    model_ms: u64,
}

fn load(shallow: bool) -> Result<Loaded, String> {
    let started = std::time::Instant::now();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page = records::scan(COLLECTION, cursor.as_deref(), PAGE)
            .map_err(|error| format!("reading the marbles book: {error:?}"))?;
        for entry in page.entries {
            if entry.deleted {
                continue;
            }
            entries.push((entry.key.id, entry.body));
        }
        match page.next {
            Some(next) if Some(next.as_str()) != cursor.as_deref() => cursor = Some(next),
            _ => {
                let scan_ms = started.elapsed().as_millis() as u64;
                let model_started = std::time::Instant::now();
                let book = if shallow {
                    model::issues_only(&entries)
                } else {
                    model::book(&entries)
                };
                return Ok(Loaded {
                    book,
                    entries: entries.len(),
                    scan_ms,
                    model_ms: model_started.elapsed().as_millis() as u64,
                });
            }
        }
    }
    let scan_ms = started.elapsed().as_millis() as u64;
    let model_started = std::time::Instant::now();
    let book = if shallow {
        model::issues_only(&entries)
    } else {
        model::book(&entries)
    };
    Ok(Loaded {
        book,
        entries: entries.len(),
        scan_ms,
        model_ms: model_started.elapsed().as_millis() as u64,
    })
}

fn answer(response_out: ResponseOutparam, status: u16, content_type: &str, body: &[u8]) {
    answer_with(response_out, status, content_type, body, &[]);
}

fn answer_with(
    response_out: ResponseOutparam,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: &[(&str, String)],
) {
    let mut list = vec![("content-type".to_string(), content_type.as_bytes().to_vec())];
    for (name, value) in headers {
        list.push((name.to_string(), value.as_bytes().to_vec()));
    }
    let fields = Fields::from_list(&list).unwrap_or_else(|_| Fields::new());
    let _ = fields.set("cache-control", &[b"no-store".to_vec()]);
    let response = OutgoingResponse::new(fields);
    if response.set_status_code(status).is_err() {
        return;
    }
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
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
                if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    out.push(byte);
                    index += 3;
                } else {
                    out.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
