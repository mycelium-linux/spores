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
    /// `claim <id>`: the id to bind to a ring.
    claim_id: Option<String>,
    /// `claim --handle N`: a specific scan handle, for when several rings are
    /// in range and the wearer knows which one is which.
    handle: Option<u32>,
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
            claim_id: None,
            handle: None,
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
                "--handle" => {
                    args.handle = Some(
                        value()?
                            .parse()
                            .map_err(|_| format!("--handle: {:?} is not a number", raw[index]))?,
                    )
                }
                "--help" | "-h" => {
                    return Err(usage());
                }
                // A bare second token is the id for `claim`.
                other if !other.starts_with("--") && args.claim_id.is_none() => {
                    args.claim_id = Some(other.to_string());
                }
                other => return Err(format!("unknown flag {other:?}\n{}", usage())),
            }
            index += 1;
        }
        match args.command.as_str() {
            "watch" | "battery" | "steps" | "probe" => Ok(args),
            "claim" if args.claim_id.is_some() => Ok(args),
            "claim" => Err(format!(
                "claim needs an id, e.g. `colmi-ring claim ring-left`\n{}",
                usage()
            )),
            other => Err(format!("unknown command {other:?}\n{}", usage())),
        }
    }
}

fn usage() -> String {
    "usage: colmi-ring <watch|battery|steps|probe> [--wearer W] [--device ID] \
     [--scan-ms N] [--hold-ms N] [--day N] [--streams hr,spo2,steps]\n\
     \x20      colmi-ring claim <id> [--handle N] [--scan-ms N]"
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
    // Two identical rings are told apart by what they answer, not by their
    // name; `probe` reports one line per ring before a session picks one.
    if args.command == "probe" {
        return probe(&devices, &args);
    }
    if args.command == "claim" {
        return claim(&devices, &args);
    }
    // Two rings advertise the same name and expose no serial. Picking the
    // first match would bind one ring's body data to the other's identity, so
    // an unselected session refuses instead.
    if args.device.is_none() {
        let matching: Vec<_> = devices
            .iter()
            .filter(|device| name_matches(device))
            .collect();
        if matching.len() > 1 {
            return Err(format!(
                "{} rings match by name and none is selected; pass --device <claimed-id>, or run `colmi-ring claim <id>` once per ring",
                matching.len()
            ));
        }
    }
    let target = find_target(&devices, &args)
        .ok_or_else(|| format!("no COLMI ring found among {} device(s)", devices.len()))?;
    let handle = target.handle;
    let advertised_name = target.name.clone().unwrap_or_default();
    let claimed = target.device_id.clone();

    let link = ble::connect(handle).map_err(ble_err)?;
    let result = run_connected(link, &args, &advertised_name, claimed);
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

/// Connect to every candidate ring in turn and report what each one says.
///
/// This is the honest answer to "which ring is which": the advertised name is
/// the same for both, the platform address is a host-local locator the Spore
/// never sees, and the only per-unit fact available is what the ring answers
/// over its own service. Duplicate serials are reported, not averaged away.
fn probe(devices: &[soil::ble::types::Device], args: &Args) -> Result<(), String> {
    let mut rings = Vec::new();
    let mut serials: Vec<String> = Vec::new();

    for device in devices {
        if !is_candidate(device, args) {
            continue;
        }
        let entry = match probe_one(device, &args.wearer) {
            Ok(entry) => {
                if let Some(serial) = entry["serial"].as_str().filter(|s| !s.is_empty()) {
                    serials.push(serial.to_string());
                }
                entry
            }
            Err(error) => json!({
                "handle": device.handle,
                "name": device.name,
                "error": error,
            }),
        };
        rings.push(entry);
    }

    let mut duplicates = Vec::new();
    for serial in &serials {
        if serials.iter().filter(|other| *other == serial).count() > 1
            && !duplicates.contains(serial)
        {
            duplicates.push(serial.clone());
        }
    }

    let now = time::monotonic_ms().map_err(time_err)?;
    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({
            "ok": true,
            "command": "probe",
            "rings": rings,
            "distinct": duplicates.is_empty(),
            "duplicates": duplicates,
            "wearer": args.wearer.clone(),
            "updated_at_monotonic_ms": now,
        });
    });
    Ok(())
}

/// Bind one ring to an id, once. With several identical rings in range the
/// wearer either says which handle, or leaves exactly one unclaimed ring.
fn claim(devices: &[soil::ble::types::Device], args: &Args) -> Result<(), String> {
    let id = args
        .claim_id
        .as_deref()
        .ok_or_else(|| "claim needs an id".to_string())?;

    let candidates: Vec<&soil::ble::types::Device> = match args.handle {
        Some(handle) => devices
            .iter()
            .filter(|device| device.handle == handle)
            .collect(),
        None => devices
            .iter()
            .filter(|device| name_matches(device))
            .collect(),
    };
    if candidates.is_empty() {
        return Err(match args.handle {
            Some(handle) => {
                format!("no ring with handle {handle}; run `probe` to see what is in range")
            }
            None => "no ring found; run `probe` to see what is in range".to_string(),
        });
    }

    let target = match args.handle {
        Some(_) => candidates[0],
        None => {
            let unclaimed: Vec<_> = candidates
                .iter()
                .filter(|device| device.device_id.is_none())
                .copied()
                .collect();
            match unclaimed.len() {
                1 => unclaimed[0],
                0 => {
                    return Err(format!(
                        "all {} matching ring(s) are already claimed; pass --handle <N> to re-claim one",
                        candidates.len()
                    ));
                }
                many => {
                    return Err(format!(
                        "{many} unclaimed rings match; put the others out of range, or pass --handle <N> (probe lists handles)"
                    ));
                }
            }
        }
    };

    ble::claim(target.handle, id).map_err(ble_err)?;
    let now = time::monotonic_ms().map_err(time_err)?;
    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({
            "ok": true,
            "command": "claim",
            "device_id": id,
            "handle": target.handle,
            "name": target.name,
            "rssi": target.rssi,
            "updated_at_monotonic_ms": now,
        });
    });
    Ok(())
}

fn name_matches(device: &soil::ble::types::Device) -> bool {
    device
        .name
        .as_deref()
        .is_some_and(|name| name.to_ascii_lowercase().contains("colmi"))
}

fn is_candidate(device: &soil::ble::types::Device, args: &Args) -> bool {
    if let Some(wanted) = &args.device {
        return device.handle.to_string() == *wanted
            || device
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(wanted));
    }
    device
        .name
        .as_deref()
        .is_some_and(|name| name.to_ascii_lowercase().contains("colmi"))
}

fn probe_one(device: &soil::ble::types::Device, wearer: &str) -> Result<Value, String> {
    let link = ble::connect(device.handle).map_err(ble_err)?;
    let result = (|| {
        // This ring answers about one GATT round trip per connection and then
        // stops responding; a second read times out, and an immediate
        // reconnect times out too. The identity read therefore goes first,
        // and the rest are best-effort. Nothing here blurs "no answer" into
        // "no data": each read reports its own outcome.
        let read = |uuid: &str| match ble::read(link, uuid, 64) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes)
                    .trim_end_matches('\0')
                    .trim()
                    .to_string();
                let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                json!({"ok": true, "len": bytes.len(), "hex": hex, "value": text})
            }
            Err(error) => json!({"ok": false, "error": ble_err(error)}),
        };

        let serial = read(proto::SERIAL_NUMBER);
        let reads = json!({
            "serial": serial,
            "model": read(proto::MODEL_NUMBER),
            "hardware": read(proto::HARDWARE_REVISION),
            "firmware": read(proto::FIRMWARE_REVISION),
        });

        let services = ble::services(link).map_err(ble_err)?;
        let has = |uuid: &str| {
            services
                .iter()
                .flat_map(|service| service.characteristics.iter())
                .any(|characteristic| characteristic.uuid.eq_ignore_ascii_case(uuid))
        };
        let device_info = json!({
            "model": has(proto::MODEL_NUMBER),
            "serial": has(proto::SERIAL_NUMBER),
            "hardware": has(proto::HARDWARE_REVISION),
            "firmware": has(proto::FIRMWARE_REVISION),
        });
        let uart = json!({
            "service": services
                .iter()
                .any(|service| service.uuid.eq_ignore_ascii_case(proto::UART_SERVICE)),
            "write": has(proto::UART_WRITE),
            "notify": has(proto::UART_NOTIFY),
        });
        let service_uuids: Vec<&str> = services
            .iter()
            .map(|service| service.uuid.as_str())
            .collect();

        // The update channel is a different service from the command channel,
        // so report what it actually exposes rather than assuming the flasher's
        // UUIDs are wired the same way on this variant.
        let ota_service = services
            .iter()
            .find(|service| service.uuid.eq_ignore_ascii_case(proto::OTA_SERVICE));
        let ota = json!({
            "service": ota_service.is_some(),
            "write": has(proto::OTA_WRITE),
            "notify": has(proto::OTA_NOTIFY),
            "characteristics": ota_service
                .map(|service| {
                    service
                        .characteristics
                        .iter()
                        .map(|characteristic| json!({
                            "uuid": characteristic.uuid,
                            "readable": characteristic.readable,
                            "writable": characteristic.writable,
                            "notifiable": characteristic.notifiable,
                        }))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        });

        let text = |read: &Value| read["value"].as_str().unwrap_or_default().to_string();
        let serial_text = text(&reads["serial"]);
        let model = text(&reads["model"]);
        let firmware = text(&reads["firmware"]);
        let hardware = text(&reads["hardware"]);

        let battery = request_battery(link).ok();

        // Battery is device state, not a body signal; only a ring with an
        // identity can own the record.
        let identity = device
            .device_id
            .clone()
            .or_else(|| watch_core::device_id(&serial_text, None));
        if let (Some(identity), Some(battery)) = (identity, battery) {
            upsert_device_state(&identity, &model, &firmware, battery.percent, wearer)?;
        }

        Ok(json!({
            "handle": device.handle,
            "name": device.name,
            "rssi": device.rssi,
            "device_id": device.device_id,
            "serial": serial_text,
            "model": model,
            "firmware": firmware,
            "hardware": hardware,
            "battery_percent": battery.map(|battery| battery.percent),
            "charging": battery.map(|battery| battery.charging),
            "device_info": device_info,
            "reads": reads,
            "uart": uart,
            "ota": ota,
            "services": service_uuids,
        }))
    })();
    let _ = ble::disconnect(link);
    result
}

fn run_connected(
    link: Link,
    args: &Args,
    advertised_name: &str,
    claimed: Option<String>,
) -> Result<(), String> {
    let model =
        read_string(link, proto::MODEL_NUMBER).unwrap_or_else(|_| advertised_name.to_string());
    let firmware = read_string(link, proto::FIRMWARE_REVISION).unwrap_or_default();
    let hardware = read_string(link, proto::HARDWARE_REVISION).unwrap_or_default();
    let serial = read_string(link, proto::SERIAL_NUMBER).unwrap_or_default();

    // Identity comes from the device, never from the rotating BLE address. A
    // ring that exposes no serial needs `--device`; inventing one from the
    // platform handle would split the same ring into a new device per host.
    let device_id = claimed
        .or_else(|| watch_core::device_id(&serial, None))
        .ok_or_else(|| {
            "this ring has no claimed id and returned an empty DIS serial; run `colmi-ring claim <id>` once with it selected".to_string()
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
