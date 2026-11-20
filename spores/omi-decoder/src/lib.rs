//! Resident Omi decoder.
//!
//! Omi publishes `omi.audio.batch.v1` frames carrying 3-byte-framed Opus
//! packets. This Spore owns the session state machine: it decodes each packet
//! through `soil:codec`, transcribes bounded PCM windows through
//! `soil:inference`, writes one `audio.wav`, and publishes `omi.capture.v1` for
//! Scribe. libopus, the STT route, VFS bytes and Unibus transport stay in the
//! host.

use std::cell::RefCell;

use base64::Engine;
use serde_json::{Value, json};
use soil::codec::codec;
use soil::inference::inference;
use soil::time::time;
use soil::unibus::bus;
use soil::vfs_mut::vfs_mut;

wit_bindgen::generate!({ path: "wit", world: "omi-decoder", generate_all });

const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;
const STT_CHUNK_FRAMES: usize = SAMPLE_RATE as usize * 10 * 60;
const MAX_PCM: usize = 160 * 1024 * 1024;
const DEFAULT_IDLE_MS: u64 = 20_000;
const DEFAULT_MODEL: &str = "scribe/stt";

thread_local! {
    static SNAPSHOT: RefCell<Value> = const { RefCell::new(Value::Null) };
}

struct Component;
export!(Component);

#[derive(Debug, Clone)]
struct Args {
    command: String,
    idle_ms: u64,
    once: bool,
    model: String,
    hold_ms: u64,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "watch".into(),
            idle_ms: DEFAULT_IDLE_MS,
            once: false,
            model: DEFAULT_MODEL.into(),
            hold_ms: 0,
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
            let flag = raw[index].clone();
            index += 1;
            match flag.as_str() {
                "--idle-ms" => {
                    args.idle_ms = raw
                        .get(index)
                        .ok_or_else(|| "--idle-ms needs a value".to_string())?
                        .parse()
                        .map_err(|_| "--idle-ms must be a number".to_string())?;
                    index += 1;
                }
                "--once" => args.once = true,
                "--model" => {
                    args.model = raw
                        .get(index)
                        .cloned()
                        .ok_or_else(|| "--model needs a value".to_string())?;
                    index += 1;
                }
                "--hold-ms" => {
                    args.hold_ms = raw
                        .get(index)
                        .ok_or_else(|| "--hold-ms needs a value".to_string())?
                        .parse()
                        .map_err(|_| "--hold-ms must be a number".to_string())?;
                    index += 1;
                }
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(args)
    }
}

#[derive(Default)]
struct Session {
    id: String,
    date: Option<String>,
    started_at_ms: Option<u64>,
    ended_at_ms: Option<u64>,
    last_seen_ms: u64,
    pcm: Vec<u8>,
    packets: u64,
    batches: u64,
}

impl Session {
    fn accepts(&self, id: &str) -> bool {
        self.id == id
    }

    fn append(&mut self, pcm_bytes: &[u8], issued_at_ms: u64) {
        if self.started_at_ms.is_none() {
            self.started_at_ms = Some(issued_at_ms);
        }
        self.ended_at_ms = Some(issued_at_ms);
        self.last_seen_ms = now_ms().unwrap_or(0);
        if self.date.is_none() {
            self.date = Some(date_from_ms(issued_at_ms));
        }
        if self.pcm.len() < MAX_PCM {
            let room = MAX_PCM - self.pcm.len();
            self.pcm.extend_from_slice(&pcm_bytes[..pcm_bytes.len().min(room)]);
        }
    }
}

struct Counters {
    batches: u64,
    packets: u64,
    captures: u64,
    failed: u64,
    dropped_bytes: u64,
}

impl Counters {
    fn new() -> Self {
        Self {
            batches: 0,
            packets: 0,
            captures: 0,
            failed: 0,
            dropped_bytes: 0,
        }
    }

    fn snapshot(&self) -> Value {
        json!({
            "batches": self.batches,
            "packets": self.packets,
            "captures": self.captures,
            "failed": self.failed,
            "dropped_bytes": self.dropped_bytes,
        })
    }
}

fn finish(value: Value) {
    SNAPSHOT.with(|slot| *slot.borrow_mut() = value);
}

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<(), String> {
        let args = Args::parse(&args)?;
        if !matches!(args.command.as_str(), "watch" | "decode") {
            return Err(format!("unknown command {}; expected watch or decode", args.command));
        }
        let mut counters = Counters::new();
        let mut session: Option<Session> = None;
        let mut cursor: Option<String> = None;
        let started = now_ms()?;
        loop {
            let page = bus::poll(&["omi.audio.batch.v1".into()], cursor.as_deref(), 32)
                .map_err(|error| format!("unibus poll: {error:?}"))?;
            for frame in page.frames {
                let issued = frame.issued_at_ms;
                let batch: Value = match serde_json::from_slice(&frame.body) {
                    Ok(batch) => batch,
                    Err(error) => {
                        counters.failed += 1;
                        eprintln!("omi-decoder: batch JSON: {error}");
                        continue;
                    }
                };
                let id = batch
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| "omi-unknown".to_string());
                if session.as_ref().is_some_and(|session| !session.accepts(&id)) {
                    if let Some(previous) = session.take() {
                        finalize(previous, &args.model, &mut counters);
                    }
                }
                let current = session.get_or_insert_with(|| Session {
                    id: id.clone(),
                    ..Session::default()
                });
                match decode_batch(current, &batch) {
                    Ok((packets, bytes)) => {
                        counters.batches += 1;
                        counters.packets += packets;
                        current.packets += packets;
                        current.batches += 1;
                        current.append(&bytes, issued);
                    }
                    Err(error) => {
                        counters.failed += 1;
                        eprintln!("omi-decoder: {id}: {error}");
                    }
                }
            }
            cursor = page.next;
            let now = now_ms()?;
            if let Some(current) = session.as_ref() {
                if !current.pcm.is_empty()
                    && now.saturating_sub(current.last_seen_ms) >= args.idle_ms
                {
                    if let Some(previous) = session.take() {
                        finalize(previous, &args.model, &mut counters);
                    }
                }
            }
            if args.once || (args.hold_ms > 0 && now.saturating_sub(started) >= args.hold_ms) {
                break;
            }
            let _ = time::tick(250);
        }
        if let Some(previous) = session {
            finalize(previous, &args.model, &mut counters);
        }
        finish(json!({
            "ok": true,
            "command": args.command,
            "counters": counters.snapshot(),
        }));
        Ok(())
    }

    fn snapshot() -> Result<Vec<u8>, String> {
        let value = SNAPSHOT.with(|slot| slot.borrow().clone());
        serde_json::to_vec(&value).map_err(|error| error.to_string())
    }
}

fn decode_batch(session: &mut Session, batch: &Value) -> Result<(u64, Vec<u8>), String> {
    let frames = batch
        .get("frames")
        .and_then(Value::as_array)
        .ok_or_else(|| "batch has no frames".to_string())?;
    let mut packets = 0u64;
    let mut pcm = Vec::new();
    for frame in frames {
        let encoded = frame
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| "frame has no data".to_string())?;
        let packet = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("frame base64: {error}"))?;
        if packet.is_empty() {
            continue;
        }
        let decoded = codec::decode(&session.id, &packet, SAMPLE_RATE, CHANNELS)
            .map_err(|error| format!("codec decode: {error:?}"))?;
        pcm.extend_from_slice(&decoded);
        packets += 1;
    }
    Ok((packets, pcm))
}

fn finalize(session: Session, model: &str, counters: &mut Counters) {
    if session.pcm.is_empty() {
        return;
    }
    let capture_id = sanitize(&session.id)
        .unwrap_or_else(|_| format!("omi-{}", session.started_at_ms.unwrap_or(0)));
    let date = session.date.clone().unwrap_or_else(|| "undated".into());
    let root = format!("captures/omi/{date}/{capture_id}");
    if let Err(error) = mkdir_recursive(&root) {
        counters.failed += 1;
        eprintln!("omi-decoder: {root}: {error}");
        return;
    }
    let wav = wav_mono(&session.pcm, SAMPLE_RATE);
    let audio = match vfs_mut::write(
        &format!("{root}/audio.wav"),
        &wav,
        "audio/wav",
        None,
        None,
    ) {
        Ok(result) => result,
        Err(error) => {
            counters.failed += 1;
            eprintln!("omi-decoder: write {root}/audio.wav: {error:?}");
            return;
        }
    };
    let transcript = match transcribe(&session.pcm, model) {
        Ok(transcript) => transcript,
        Err(error) => {
            counters.failed += 1;
            eprintln!("omi-decoder: transcribe {capture_id}: {error}");
            return;
        }
    };
    if transcript
        .get("text")
        .and_then(Value::as_str)
        .is_none_or(|text| text.trim().is_empty())
    {
        counters.failed += 1;
        eprintln!("omi-decoder: {capture_id}: empty transcript");
        return;
    }
    let event = json!({
        "capture_id": capture_id,
        "source": "omi",
        "date": date,
        "started_at_ms": session.started_at_ms,
        "ended_at_ms": session.ended_at_ms,
        "title": Value::Null,
        "summary_md": "",
        "actions": [],
        "transcript": transcript,
        "audio_content_id": audio.content_id,
    });
    let body = match serde_json::to_vec(&event) {
        Ok(body) => body,
        Err(error) => {
            counters.failed += 1;
            eprintln!("omi-decoder: capture JSON: {error}");
            return;
        }
    };
    match bus::publish("omi.capture.v1", &body, None, 60_000) {
        Ok(_) => {
            counters.captures += 1;
            eprintln!(
                "omi-decoder: {capture_id}: {} packets, {} batches, {} bytes",
                session.packets,
                session.batches,
                session.pcm.len()
            );
        }
        Err(error) => {
            counters.failed += 1;
            eprintln!("omi-decoder: publish {capture_id}: {error:?}");
        }
    }
}

fn transcribe(pcm: &[u8], model: &str) -> Result<Value, String> {
    let mut segments = Vec::new();
    let mut words = Vec::new();
    let mut text_parts = Vec::new();
    let chunk_bytes = STT_CHUNK_FRAMES * 2;
    let mut offset_frames = 0usize;
    for chunk in pcm.chunks(chunk_bytes) {
        if chunk.is_empty() {
            continue;
        }
        let offset_ms = (offset_frames as f64 / SAMPLE_RATE as f64 * 1000.0) as u64;
        let request = inference::TranscribeRequest {
            model: model.to_string(),
            sample_rate: SAMPLE_RATE,
            pcm16: chunk.to_vec(),
            language: None,
            prompt: None,
            timestamps: true,
            diarize: true,
        };
        let response = inference::transcribe(&request)
            .map_err(|error| format!("inference: {error:?}"))?;
        if !response.text.trim().is_empty() {
            text_parts.push(response.text.trim().to_string());
        }
        for segment in response.segments {
            let start = segment.start_ms as u64 + offset_ms;
            let end = segment.end_ms as u64 + offset_ms;
            let segment_words: Vec<Value> = segment
                .words
                .iter()
                .map(|word| {
                    let word_start = word.start_ms as u64 + offset_ms;
                    let word_end = word.end_ms as u64 + offset_ms;
                    let value = json!({
                        "text": word.text,
                        "start": word_start as f64 / 1000.0,
                        "end": word_end as f64 / 1000.0,
                        "speaker": word.speaker.clone().or_else(|| segment.speaker.clone()),
                    });
                    words.push(value.clone());
                    value
                })
                .collect();
            segments.push(json!({
                "text": segment.text,
                "start": start as f64 / 1000.0,
                "end": end as f64 / 1000.0,
                "speaker": segment.speaker,
                "words": segment_words,
            }));
        }
        offset_frames += chunk.len() / 2;
    }
    Ok(json!({
        "text": text_parts.join(" "),
        "segments": segments,
        "words": words,
    }))
}

fn now_ms() -> Result<u64, String> {
    time::monotonic_ms().map_err(|error| format!("time: {error:?}"))
}

fn sanitize(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return Err("invalid capture id".into());
    }
    if value
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')))
    {
        return Err("capture id has unsupported characters".into());
    }
    Ok(value.to_string())
}

fn date_from_ms(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's civil-from-days, with the Unix epoch as day zero.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

fn mkdir_recursive(path: &str) -> Result<(), String> {
    let mut prefix = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        match vfs_mut::mkdir(&prefix) {
            Ok(()) | Err(vfs_mut::Error::AlreadyExists) => {}
            Err(error) => return Err(format!("mkdir {prefix}: {error:?}")),
        }
    }
    Ok(())
}

fn wav_mono(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_epoch_days() {
        assert_eq!(date_from_ms(0), "1970-01-01");
        assert_eq!(date_from_ms(1_700_000_000_000), "2023-11-14");
    }

    #[test]
    fn wav_has_riff_header() {
        let wav = wav_mono(&[0, 0, 1, 0], 16_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }
}
