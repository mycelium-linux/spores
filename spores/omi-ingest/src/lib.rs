//! Receive one Omi capture bundle over a host-owned WSS session.
//!
//! Wire format, all integers big-endian:
//!
//! ```text
//! "OMI1"
//! repeat:
//!   u16 path length
//!   path bytes (capture-relative under captures/)
//!   u64 data length
//!   data bytes
//! u16 zero                         (end of bundle)
//! ```
//!
//! Each file is streamed straight into `soil:vfs-mut`; audio is never held in
//! guest memory. After the last file the guest materializes the Scribe bundle
//! (`capture.json`, `transcript.json`, `summary.md`, `actions.json`) from the
//! uploaded `event.json`, then answers `OK`.

wit_bindgen::generate!({ path: "wit", world: "omi-ingest", generate_all });

use serde_json::{json, Value};
use soil::streams::duplex;
use soil::time::time;
use soil::vfs_mut::vfs_mut;

const MAGIC: &[u8; 4] = b"OMI1";
const MAX_PATH: usize = 1024;
const MAX_FILE: u64 = 2 * 60 * 60 * 16_000 * 2 + 4096;
const CHUNK: usize = 64 * 1024;

struct Component;
export!(Component);

impl Guest for Component {
    fn run(_args: Vec<String>) -> Result<(), String> {
        loop {
            let Some(session) = duplex::accept("wss").map_err(|error| format!("accept: {error:?}"))?
            else {
                time::tick(50).map_err(|error| format!("tick: {error:?}"))?;
                continue;
            };
            if let Err(error) = receive(session) {
                eprintln!("omi-ingest: {error}");
            }
        }
    }
}

fn receive(mut session: duplex::Session) -> Result<(), String> {
    let mut reader = Reader::new(&mut session);
    let mut magic = [0u8; 4];
    if !reader.read_exact(&mut magic)? {
        return Err("empty stream".into());
    }
    if &magic != MAGIC {
        return Err("bad magic".into());
    }

    let mut event: Option<Vec<u8>> = None;
    let mut capture_dir: Option<String> = None;
    loop {
        let Some(path_len) = reader.read_u16()? else {
            break;
        };
        let path_len = path_len as usize;
        if path_len == 0 {
            break;
        }
        if path_len > MAX_PATH {
            return Err(format!("bad path length {path_len}"));
        }
        let mut path_bytes = vec![0u8; path_len];
        if !reader.read_exact(&mut path_bytes)? {
            return Err("truncated path".into());
        }
        let path = String::from_utf8(path_bytes).map_err(|_| "path is not utf-8")?;
        if !safe_path(&path) {
            return Err(format!("unsafe path {path:?}"));
        }
        let len = reader
            .read_u64()?
            .ok_or_else(|| format!("missing data length for {path}"))?;
        if len > MAX_FILE {
            return Err(format!("file {path} is too large: {len}"));
        }
        if let Some(dir) = path.rsplit_once('/').map(|(dir, _)| dir) {
            capture_dir.get_or_insert_with(|| dir.to_string());
        }
        let media = media_type(&path);
        let writer = vfs_mut::begin_write(&path, media, Some(len), None, None)
            .map_err(|error| format!("begin-write {path}: {error:?}"))?;
        let mut remaining = len;
        let mut kept = Vec::new();
        while remaining > 0 {
            let want = remaining.min(CHUNK as u64) as usize;
            let chunk = reader
                .read_some(want)?
                .ok_or_else(|| format!("truncated data for {path}"))?;
            if path.ends_with("/event.json") && kept.len() < 512 * 1024 {
                let left = (512 * 1024 - kept.len()).min(chunk.len());
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
            event = Some(kept);
        }
    }

    if let (Some(event), Some(dir)) = (event, capture_dir) {
        materialize(&dir, &event)?;
    }
    session
        .write(b"OK\n")
        .map_err(|error| format!("ack: {error:?}"))?;
    session.close().map_err(|error| format!("close: {error:?}"))?;
    Ok(())
}

fn materialize(dir: &str, event_bytes: &[u8]) -> Result<(), String> {
    let wrapper: Value = serde_json::from_slice(event_bytes).map_err(|error| error.to_string())?;
    let event = wrapper.get("event").cloned().unwrap_or(wrapper);
    let transcript = event.get("transcript").cloned().unwrap_or_else(|| json!({}));
    let text = transcript
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let title = event
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            let first = text.split(". ").next().unwrap_or(text);
            (!first.is_empty()).then(|| first.chars().take(72).collect::<String>())
        })
        .unwrap_or_else(|| "Omi capture".to_string());
    let started = event
        .get("started_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let ended = event
        .get("ended_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(started);
    let source = event
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("omi")
        .to_string();
    let id = event
        .get("capture_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| dir.rsplit('/').next().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let summary = event
        .get("summary_md")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("# {title}\n\n"));
    let actions = event
        .get("actions")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let capture = json!({
        "id": id,
        "source": source,
        "title": title,
        "started_at_ms": started,
        "ended_at_ms": ended,
        "date": event.get("date").cloned().unwrap_or(Value::Null),
        "audio": format!("{dir}/audio.wav"),
    });

    write_json(&format!("{dir}/transcript.json"), &transcript)?;
    write_json(&format!("{dir}/actions.json"), &actions)?;
    write_json(&format!("{dir}/capture.json"), &capture)?;
    vfs_mut::write(
        &format!("{dir}/summary.md"),
        summary.as_bytes(),
        "text/markdown; charset=utf-8",
        None,
        None,
    )
    .map_err(|error| format!("write summary: {error:?}"))?;
    Ok(())
}

fn write_json(path: &str, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    vfs_mut::write(path, &bytes, "application/json", None, None)
        .map_err(|error| format!("write {path}: {error:?}"))?;
    Ok(())
}

fn safe_path(path: &str) -> bool {
    path.starts_with("captures/")
        && !path.contains("..")
        && !path.contains('\\')
        && !path.ends_with('/')
        && path.len() <= MAX_PATH
}

fn media_type(path: &str) -> &'static str {
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

struct Reader<'a> {
    session: &'a mut duplex::Session,
    buffer: Vec<u8>,
    offset: usize,
}

impl<'a> Reader<'a> {
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
            .read(CHUNK as u32)
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
