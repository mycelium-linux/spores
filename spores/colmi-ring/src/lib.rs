//! The COLMI ring Spore: BLE requests in, typed records out.
//!
//! The ring is a different device from a watch: no screen, no media service,
//! one UART characteristic pair, and measurements that are request/response
//! rather than subscriptions. What it shares with the PineTime is the record
//! envelope and the identity rule, which is exactly what `watch-core` owns.
//!
//! The radio transport and the per-device lease stay in `soil-ble-host`; this
//! module owns the QRing session, the wearer identity, and the signal envelope.

use std::cell::RefCell;

use colmi as proto;
use serde_json::{json, Value};
use soil::ble::ble;
use soil::ble::types::{Event, Link};
use soil::records::records::{self, Entry, Key};
use soil::time::time;
use watch_core::{Stream, WatchProfile};

wit_bindgen::generate!({ path: "wit", world: "colmi-ring", generate_all });

const SOURCE_SPORE: &str = "dev.fungos.colmi-ring";
const PROFILE: proto::Profile = proto::Profile;
const MODEL_VERSION: &str = "colmi-ring-spore@0.1.0";
/// How long to wait for a reply to a request. The ring answers in tens of
/// milliseconds; a second is generous and keeps a dead link from hanging.
const REPLY_WAIT_MS: u32 = 1_000;

thread_local! {
    static SNAPSHOT: RefCell<Value> = RefCell::new(Value::Null);
    static SEQUENCE: RefCell<u64> = RefCell::new(0);
}

struct Component;
export!(Component);

#[derive(Debug, Clone)]
struct Args {
    command: String,
    wearer: String,
    device: Option<String>,
    scan_ms: u32,
    hold_ms: u64,
    day_offset: u8,
    streams: Vec<Stream>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "watch".to_string(),
            wearer: "owner".to_string(),
            device: None,
            scan_ms: 8_000,
            hold_ms: 5_000,
            day_offset: 0,
            streams: PROFILE.streams().to_vec(),
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
                "--wearer" => args.wearer = value()?,
                "--device" => args.device = Some(value()?),
                "--scan-ms" => {
                    args.scan_ms = value()?
                        .parse()
                        .map_err(|_| format!("--scan-ms: {:?} is not a number", raw[index]))?
                }
                "--hold-ms" => {
                    args.hold_ms = value()?
                        .parse()
                        .map_err(|_| format!("--hold-ms: {:?} is not a number", raw[index]))?
                }
                "--day" => {
                    args.day_offset = value()?
                        .parse()
                        .map_err(|_| format!("--day: {:?} is not a number", raw[index]))?
                }
                "--streams" => args.streams = Stream::parse_from(&value()?, PROFILE.streams())?,
                "--help" | "-h" => {
                    return Err(usage());
                }
                other => return Err(format!("unknown flag {other:?}\n{}", usage())),
            }
            index += 1;
        }
        match args.command.as_str() {
            "watch" | "battery" | "steps" => Ok(args),
            other => Err(format!("unknown command {other:?}\n{}", usage())),
        }
    }
}

fn usage() -> String {
    "usage: colmi-ring <watch|battery|steps> [--wearer W] [--device ID] \
     [--scan-ms N] [--hold-ms N] [--day N] [--streams hr,spo2,steps]"
        .to_string()
}

impl Guest for Component {
    fn run(raw: Vec<String>) -> Result<(), String> {
        let result = run_inner(&raw);
        if let Err(error) = &result {
            set_error(error);
        }
        result
    }

    fn snapshot() -> Result<Vec<u8>, String> {
        SNAPSHOT.with(|slot| {
            serde_json::to_vec(&*slot.borrow()).map_err(|error| format!("snapshot JSON: {error}"))
        })
    }
}

fn run_inner(raw: &[String]) -> Result<(), String> {
    let args = Args::parse(raw)?;

    let devices = ble::scan(args.scan_ms).map_err(ble_err)?;
    let target = find_target(&devices, &args)
        .ok_or_else(|| format!("no COLMI ring found among {} device(s)", devices.len()))?;
    let handle = target.handle;
    let advertised_name = target.name.clone().unwrap_or_default();

    let link = ble::connect(handle).map_err(ble_err)?;
    let result = run_connected(link, &args, &advertised_name);
    // A Spore should not keep the radio leased after a failed command either.
    let _ = ble::disconnect(link);
    result
}

fn find_target<'a>(
    devices: &'a [soil::ble::types::Device],
    args: &Args,
) -> Option<&'a soil::ble::types::Device> {
    devices.iter().find(|device| {
        if let Some(wanted) = &args.device {
            device.device_id.as_deref() == Some(wanted.as_str())
                || device.handle.to_string() == *wanted
                || device
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        } else {
            // Two identical rings advertise the same name; `--device` is how a
            // wearer says which one. Without it, the first is the honest pick.
            device
                .name
                .as_deref()
                .is_some_and(|name| name.to_ascii_lowercase().contains("colmi"))
        }
    })
}

fn run_connected(link: Link, args: &Args, advertised_name: &str) -> Result<(), String> {
    let model =
        read_string(link, proto::MODEL_NUMBER).unwrap_or_else(|_| advertised_name.to_string());
    let firmware = read_string(link, proto::FIRMWARE_REVISION).unwrap_or_default();
    let hardware = read_string(link, proto::HARDWARE_REVISION).unwrap_or_default();
    let serial = read_string(link, proto::SERIAL_NUMBER).unwrap_or_default();

    // Identity comes from the device, never from the rotating BLE address. A
    // ring that exposes no serial needs `--device`; inventing one from the
    // platform handle would split the same ring into a new device per host.
    let device_id = watch_core::device_id(&serial, args.device.as_deref()).ok_or_else(|| {
        "COLMI ring exposed no DIS serial; pass --device <provisioned-id> instead of using the BLE address"
            .to_string()
    })?;

    let mut counts: Counts = Counts::default();
    let mut battery_percent: Option<u8> = None;

    match args.command.as_str() {
        "watch" => {
            let battery = request_battery(link)?;
            battery_percent = Some(battery.percent);
            upsert_device_state(&device_id, &model, &firmware, battery.percent, &args.wearer)?;

            ble::subscribe(link, proto::UART_NOTIFY).map_err(ble_err)?;
            // The ring streams one real-time reading at a time: starting a
            // second replaces the first. Sessions therefore take a window per
            // reading, the same way the reference client polls them.
            let kinds: Vec<_> = args
                .streams
                .iter()
                .filter_map(|stream| real_time_kind(*stream))
                .collect();
            if kinds.is_empty() && !args.streams.contains(&Stream::Steps) {
                return Err(
                    "--streams selected nothing this ring can stream; it streams hr and spo2"
                        .to_string(),
                );
            }
            let window_ms = args.hold_ms / kinds.len().max(1) as u64;
            for kind in &kinds {
                let start = proto::start_real_time(*kind).map_err(proto_err)?;
                ble::write(link, proto::UART_WRITE, &start).map_err(ble_err)?;
                let result = drain(
                    link,
                    &device_id,
                    &args.wearer,
                    window_ms,
                    &args.streams,
                    &mut counts,
                );
                // Stop what this window started: a ring left streaming drains
                // its own battery after the Spore is gone.
                let stop = proto::stop_real_time(*kind).map_err(proto_err)?;
                if let Err(error) = ble::write(link, proto::UART_WRITE, &stop) {
                    println!("colmi-ring: stop {kind:?} failed: {error:?}");
                }
                result?;
            }

            if args.streams.contains(&Stream::Steps) {
                let details = read_steps(link, &device_id, &args.wearer, args.day_offset)?;
                counts.steps = details.len();
            }
        }
        "battery" => {
            let battery = request_battery(link)?;
            battery_percent = Some(battery.percent);
            upsert_device_state(&device_id, &model, &firmware, battery.percent, &args.wearer)?;
        }
        "steps" => {
            let details = read_steps(link, &device_id, &args.wearer, args.day_offset)?;
            counts.steps = details.len();
        }
        other => return Err(format!("unknown command {other:?}")),
    }

    let now = time::monotonic_ms().map_err(time_err)?;
    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({
            "ok": true,
            "command": args.command.clone(),
            "device_id": device_id.clone(),
            "model": model.clone(),
            "firmware": firmware.clone(),
            "hardware": hardware.clone(),
            "battery_percent": battery_percent,
            "wearer": args.wearer.clone(),
            "profile": PROFILE.name(),
            "streams": args.streams.iter().map(|stream| stream.as_str()).collect::<Vec<_>>(),
            "signals": counts.json(),
            "updated_at_monotonic_ms": now,
        });
    });
    Ok(())
}

/// Which real-time reading a stream maps to. Steps and battery are
/// request/response, so they have no stream command.
fn real_time_kind(stream: Stream) -> Option<proto::RealTime> {
    match stream {
        Stream::HeartRate => Some(proto::RealTime::HeartRate),
        Stream::Spo2 => Some(proto::RealTime::Spo2),
        _ => None,
    }
}

fn drain(
    link: Link,
    device_id: &str,
    wearer: &str,
    hold_ms: u64,
    streams: &[Stream],
    counts: &mut Counts,
) -> Result<(), String> {
    let start = time::monotonic_ms().map_err(time_err)?;
    let deadline = start.saturating_add(hold_ms);

    loop {
        let now = time::monotonic_ms().map_err(time_err)?;
        if now >= deadline {
            break;
        }
        let wait = (deadline - now).min(u64::from(REPLY_WAIT_MS)) as u32;
        match ble::next_event(link, wait).map_err(ble_err)? {
            Some(Event::Notification(notification)) => {
                let Some(signal) = PROFILE.decode(&notification.char_uuid, &notification.data)?
                else {
                    continue;
                };
                if !streams.contains(&signal.stream) {
                    continue;
                }
                note_signal(device_id, wearer, &signal)?;
                counts.bump(signal.stream);
            }
            Some(Event::Disconnected(reason)) => {
                return Err(format!("COLMI ring disconnected: {reason}"));
            }
            None => {}
        }
    }
    Ok(())
}

/// Ask for the battery level and wait for its reply.
fn request_battery(link: Link) -> Result<proto::Battery, String> {
    ble::subscribe(link, proto::UART_NOTIFY).map_err(ble_err)?;
    ble::write(
        link,
        proto::UART_WRITE,
        &proto::battery_request().map_err(proto_err)?,
    )
    .map_err(ble_err)?;
    let packet = wait_for(link, proto::CMD_BATTERY, "")?;
    proto::parse_battery(&packet).map_err(proto_err)
}

/// Request one day of activity and parse the pages until the ring says done.
fn read_steps(
    link: Link,
    device_id: &str,
    wearer: &str,
    day_offset: u8,
) -> Result<Vec<proto::SportDetail>, String> {
    ble::subscribe(link, proto::UART_NOTIFY).map_err(ble_err)?;
    ble::write(
        link,
        proto::UART_WRITE,
        &proto::steps_request(day_offset).map_err(proto_err)?,
    )
    .map_err(ble_err)?;

    let mut parser = proto::StepsParser::new();
    let start = time::monotonic_ms().map_err(time_err)?;
    let deadline = start.saturating_add(5_000);
    loop {
        let now = time::monotonic_ms().map_err(time_err)?;
        if now >= deadline {
            return Err(format!("no steps reply after 5 s (day {day_offset})"));
        }
        let wait = (deadline - now).min(u64::from(REPLY_WAIT_MS)) as u32;
        let Some(packet) = next_packet(link, wait)? else {
            continue;
        };
        if packet[0] != proto::CMD_GET_STEPS {
            continue;
        }
        match parser.push(&packet).map_err(proto_err)? {
            proto::StepsUpdate::Done(details) => {
                for detail in &details {
                    note_steps(device_id, wearer, detail)?;
                }
                return Ok(details);
            }
            proto::StepsUpdate::NoData => return Ok(Vec::new()),
            proto::StepsUpdate::More => {}
        }
    }
}

/// Wait for one reply whose command byte matches, skipping other traffic (a
/// running real-time stream may still be in flight).
fn wait_for(link: Link, command: u8, context: &str) -> Result<Vec<u8>, String> {
    let start = time::monotonic_ms().map_err(time_err)?;
    let deadline = start.saturating_add(u64::from(REPLY_WAIT_MS));
    loop {
        let now = time::monotonic_ms().map_err(time_err)?;
        if now >= deadline {
            return Err(format!("no reply to command {command}{context}"));
        }
        let wait = (deadline - now).min(u64::from(REPLY_WAIT_MS)) as u32;
        if let Some(packet) = next_packet(link, wait)? {
            if packet[0] == command {
                return Ok(packet);
            }
        }
    }
}

fn next_packet(link: Link, wait_ms: u32) -> Result<Option<Vec<u8>>, String> {
    match ble::next_event(link, wait_ms).map_err(ble_err)? {
        Some(Event::Notification(notification)) => Ok(Some(notification.data)),
        Some(Event::Disconnected(reason)) => Err(format!("COLMI ring disconnected: {reason}")),
        None => Ok(None),
    }
}

fn read_string(link: Link, uuid: &str) -> Result<String, String> {
    let bytes = ble::read(link, uuid, 64).map_err(ble_err)?;
    String::from_utf8(bytes)
        .map(|text| text.trim_end_matches('\0').trim().to_string())
        .map_err(|error| format!("{uuid}: not UTF-8: {error}"))
}

#[derive(Debug, Default)]
struct Counts {
    hr: u64,
    spo2: u64,
    steps: usize,
}

impl Counts {
    fn bump(&mut self, stream: Stream) {
        match stream {
            Stream::HeartRate => self.hr += 1,
            Stream::Spo2 => self.spo2 += 1,
            _ => {}
        }
    }

    fn json(&self) -> Value {
        json!({"hr": self.hr, "spo2": self.spo2, "steps": self.steps})
    }
}

fn upsert_device_state(
    device_id: &str,
    model: &str,
    firmware: &str,
    battery: u8,
    wearer: &str,
) -> Result<(), String> {
    let now = time::monotonic_ms().map_err(time_err)?;
    let body = watch_core::device_state_body(
        device_id,
        wearer,
        SOURCE_SPORE,
        model,
        firmware,
        battery,
        now,
    );
    upsert("device.state", device_id, "device.state.v1", &body)
}

/// One quarter-hour activity bucket, written as a step signal. The bucket,
/// not the day, is the observation: summing buckets happens in the reader.
fn note_steps(device_id: &str, wearer: &str, detail: &proto::SportDetail) -> Result<(), String> {
    let signal = watch_core::Signal {
        stream: Stream::Steps,
        value: json!({
            "count": detail.steps,
            "calories": detail.calories,
            "distance_m": detail.distance,
            "bucket": detail.time_index,
            "year": detail.year,
            "month": detail.month,
            "day": detail.day,
        }),
        privacy_class: "health",
        retention_class: "summary",
        quality: 1.0,
    };
    note_signal(device_id, wearer, &signal)
}

fn note_signal(device_id: &str, wearer: &str, signal: &watch_core::Signal) -> Result<(), String> {
    let now = time::monotonic_ms().map_err(time_err)?;
    let sequence = SEQUENCE.with(|cell| {
        let mut value = cell.borrow_mut();
        *value = value.saturating_add(1);
        *value
    });
    let body = watch_core::signal_body(
        device_id,
        wearer,
        SOURCE_SPORE,
        MODEL_VERSION,
        signal,
        now,
        sequence,
    );
    let signal_id = body["signal_id"].as_str().unwrap_or_default().to_string();
    upsert("signal", &signal_id, "signal.sample.v1", &body)
}

fn upsert(collection: &str, id: &str, schema: &str, body: &Value) -> Result<(), String> {
    let entry = Entry {
        key: Key {
            collection: collection.to_string(),
            id: id.to_string(),
        },
        schema: schema.to_string(),
        revision: 0,
        updated_at_ms: 0,
        origin: String::new(),
        deleted: false,
        body: serde_json::to_vec(body).map_err(|error| format!("record JSON: {error}"))?,
    };
    records::upsert(&entry).map_err(|error| format!("{collection}/{id}: {error:?}"))?;
    Ok(())
}

fn set_error(error: &str) {
    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({"ok": false, "error": error});
    });
}

fn ble_err(error: ble::Error) -> String {
    format!("ble: {error:?}")
}

fn proto_err(error: proto::ProtocolError) -> String {
    format!("qring: {error}")
}

fn time_err(error: time::Error) -> String {
    format!("time: {error:?}")
}
