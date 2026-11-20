//! The PineTime Spore: BLE reads/writes in, typed records out.
//!
//! The radio transport and the lease stay in `soil-ble-host`; this module owns
//! the InfiniTime protocol mapping, the wearer identity, and the signal
//! envelope. It has no sockets, filesystem, wall clock, or ambient authority.

use std::cell::RefCell;

mod bridge;

use bridge::{Decision, WatchResponse};
use infinitime as proto;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use soil::ble::ble;
use soil::ble::types::{Event, Link};
use soil::records::records::{self, Entry, Key};
use soil::time::time;
use soil::unibus::bus;
use watch_core::{Intent, Stream, WatchProfile};
use weather_protocol as weather;

wit_bindgen::generate!({ path: "wit", world: "pinetime-spore", generate_all });

const SOURCE_SPORE: &str = "dev.fungos.pinetime";
/// The wire this Spore speaks. A second watch is a second profile, not a fork
/// of this crate.
const PROFILE: proto::Profile = proto::Profile;
const MODEL_VERSION: &str = "pinetime-spore@0.2.0";

thread_local! {
    static SNAPSHOT: RefCell<Value> = const { RefCell::new(Value::Null) };
    static BRIDGE_SUMMARY: RefCell<Value> = const { RefCell::new(Value::Null) };
    static SEQUENCE: RefCell<u64> = const { RefCell::new(0) };
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
    category: String,
    seconds: u64,
    title: String,
    body: String,
    artist: String,
    track: String,
    album: String,
    playing: bool,
    epoch: Option<u64>,
    offset_min: i32,
    icon: String,
    narrative: String,
    distance: String,
    progress: u8,
    temp_c: Option<i16>,
    min_c: Option<i16>,
    max_c: Option<i16>,
    location: String,
    sunrise_min: i16,
    sunset_min: i16,
    forecast: Vec<proto::ForecastDay>,
    read_back: bool,
    streams: Vec<Stream>,
    /// `weather --from-record`: take every weather field from the record.
    from_record: bool,
    /// `--id ID` with `--from-record`; absent means the newest record.
    record_id: Option<String>,
    /// Weather flags the caller passed. They conflict with `--from-record`.
    weather_flags: Vec<String>,
    person: Option<String>,
    poll_ms: u32,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: "watch".to_string(),
            wearer: "owner".to_string(),
            device: None,
            scan_ms: 8_000,
            hold_ms: 5_000,
            category: "simple".to_string(),
            seconds: 300,
            title: String::new(),
            body: String::new(),
            artist: String::new(),
            track: String::new(),
            album: String::new(),
            playing: true,
            epoch: None,
            offset_min: 0,
            icon: String::new(),
            narrative: String::new(),
            distance: String::new(),
            progress: 0,
            temp_c: None,
            min_c: None,
            max_c: None,
            location: String::new(),
            sunrise_min: proto::SUN_UNKNOWN,
            sunset_min: proto::SUN_UNKNOWN,
            forecast: Vec::new(),
            read_back: false,
            streams: PROFILE.streams().to_vec(),
            from_record: false,
            record_id: None,
            weather_flags: Vec::new(),
            person: None,
            poll_ms: 500,
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
            // `--read-back` and `--from-record` are the flags that stand alone.
            if flag == "--read-back" {
                args.read_back = true;
                index += 1;
                continue;
            }
            if flag == "--from-record" {
                args.from_record = true;
                index += 1;
                continue;
            }
            let value = raw
                .get(index + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?;
            if matches!(
                flag,
                "--epoch"
                    | "--icon"
                    | "--temp-c"
                    | "--min-c"
                    | "--max-c"
                    | "--location"
                    | "--sunrise-min"
                    | "--sunset-min"
                    | "--forecast"
            ) {
                args.weather_flags.push(flag.to_string());
            }
            match flag {
                "--wearer" => args.wearer = value.clone(),
                "--device" => args.device = Some(value.clone()),
                "--scan-ms" => {
                    args.scan_ms = value
                        .parse()
                        .map_err(|_| format!("--scan-ms: {value:?} is not a number"))?
                }
                "--hold-ms" => {
                    args.hold_ms = value
                        .parse()
                        .map_err(|_| format!("--hold-ms: {value:?} is not a number"))?
                }
                "--category" => args.category = value.clone(),
                "--seconds" => {
                    args.seconds = value
                        .parse()
                        .map_err(|_| format!("--seconds: {value:?} is not a number"))?
                }
                "--title" => args.title = value.clone(),
                "--body" => args.body = value.clone(),
                "--artist" => args.artist = value.clone(),
                "--track" => args.track = value.clone(),
                "--album" => args.album = value.clone(),
                "--playing" => args.playing = matches!(value.as_str(), "true" | "1" | "yes"),
                "--streams" => {
                    args.streams = Stream::parse_from(value, PROFILE.streams())?;
                }
                "--epoch" => {
                    args.epoch = Some(
                        value
                            .parse()
                            .map_err(|_| format!("--epoch: {value:?} is not a unix timestamp"))?,
                    )
                }
                "--offset-min" => {
                    args.offset_min = value
                        .parse()
                        .map_err(|_| format!("--offset-min: {value:?} is not a number"))?
                }
                "--icon" => args.icon = value.clone(),
                "--narrative" => args.narrative = value.clone(),
                "--distance" => args.distance = value.clone(),
                "--progress" => {
                    args.progress = value
                        .parse()
                        .map_err(|_| format!("--progress: {value:?} is not a percentage"))?
                }
                "--temp-c" => args.temp_c = Some(degrees(value, "--temp-c")?),
                "--min-c" => args.min_c = Some(degrees(value, "--min-c")?),
                "--max-c" => args.max_c = Some(degrees(value, "--max-c")?),
                "--location" => args.location = value.clone(),
                "--sunrise-min" => args.sunrise_min = count(value, "--sunrise-min")?,
                "--sunset-min" => args.sunset_min = count(value, "--sunset-min")?,
                "--forecast" => args.forecast = forecast(value)?,
                "--id" => args.record_id = Some(value.clone()),
                "--person" => args.person = Some(value.clone()),
                "--poll-ms" => {
                    args.poll_ms = value
                        .parse()
                        .map_err(|_| format!("--poll-ms: {value:?} is not a number"))?
                }
                other => return Err(format!("unknown argument {other:?}")),
            }
            index += 2;
        }
        match args.command.as_str() {
            "watch" | "notify" | "timer" | "media" | "find" | "time" | "nav" | "weather" | "bridge" => {}
            other => {
                return Err(format!(
                    "unknown command {other:?}; expected watch, notify, timer, media, find, time, nav, weather, or bridge"
                ))
            }
        }
        if args.from_record && args.command != "weather" {
            return Err("--from-record only applies to weather".to_string());
        }
        if args.record_id.is_some() && !args.from_record {
            return Err("--id needs --from-record".to_string());
        }
        if args.from_record {
            if let Some(flag) = args.weather_flags.first() {
                return Err(format!(
                    "{flag} conflicts with --from-record; the record carries every weather field"
                ));
            }
        }
        Ok(args)
    }
}

#[derive(Debug, Default, Clone)]
struct Counts(std::collections::BTreeMap<&'static str, u64>);

impl Counts {
    fn bump(&mut self, stream: Stream) {
        *self.0.entry(stream.as_str()).or_default() += 1;
    }

    fn bump_key(&mut self, key: &'static str) {
        *self.0.entry(key).or_default() += 1;
    }

    fn json(&self) -> Value {
        json!(self.0)
    }
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

fn host_sleep(ms: u64) -> Result<(), String> {
    let start = time::monotonic_ms().map_err(time_err)?;
    loop {
        let now = time::monotonic_ms().map_err(time_err)?;
        if now >= start.saturating_add(ms) {
            return Ok(());
        }
        let wait = (start + ms - now).min(1000) as u32;
        let _ = time::tick(wait);
    }
}

fn run_inner(raw: &[String]) -> Result<(), String> {
    let args = Args::parse(raw)?;
    if args.command == "timer" {
        host_sleep(args.seconds.saturating_mul(1000))?;
    }

    let devices = ble::scan(args.scan_ms).map_err(ble_err)?;
    let target = find_target(&devices, &args)
        .ok_or_else(|| format!("no PineTime found among {} device(s)", devices.len()))?;
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
            // InfiniTime advertises the firmware name, not the hardware name.
            device.name.as_deref().is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                name.contains("pinetime") || name.contains("infinitime")
            })
        }
    })
}

fn run_connected(link: Link, args: &Args, advertised_name: &str) -> Result<(), String> {
    let model = read_string(link, proto::DIS_MODEL).unwrap_or_else(|_| advertised_name.to_string());
    let firmware = read_string(link, proto::DIS_FIRMWARE).unwrap_or_default();
    let serial = read_string(link, proto::DIS_SERIAL).unwrap_or_default();

    // Identity comes from the device, never from the rotating BLE address. If
    // the firmware exposes no serial, an explicit provisioning id is the only
    // honest fallback; inventing one from the platform handle would split the
    // same watch into a new device on every host.
    let device_id = watch_core::device_id(&serial, args.device.as_deref()).ok_or_else(|| {
        "PineTime exposed no DIS serial; pass --device <provisioned-id> instead of using the BLE address"
            .to_string()
    })?;

    let intent = intent_of(&args.command);
    if let Some(intent) = intent {
        if PROFILE.supports(&firmware, intent) == Some(false) {
            println!(
                "pinetime: {} is not supported by firmware {firmware:?}; writing anyway and letting the characteristic answer",
                intent.as_str()
            );
        }
    }

    let battery = read_battery(link)?;
    upsert_device_state(&device_id, &model, &firmware, battery, &args.wearer)?;

    let mut writes: Vec<Written> = Vec::new();
    let counts = match args.command.as_str() {
        "watch" => {
            subscribe(link, &args.streams)?;
            drain(
                link,
                &device_id,
                &args.wearer,
                args.hold_ms,
                &model,
                &firmware,
                &args.streams,
            )?
        }
        "notify" => {
            let category =
                proto::AlertCategory::parse(&args.category).map_err(|error| error.to_string())?;
            let packet = proto::notification_packet(category.as_u8(), &args.title, &args.body);
            proto::ensure_notification_packet(&packet).map_err(|error| error.to_string())?;
            writes.push(write_characteristic(
                link,
                proto::NEW_ALERT,
                &packet,
                args.read_back,
            )?);
            Counts::default()
        }
        "timer" => {
            let title = if args.title.is_empty() { "Timer" } else { &args.title };
            let body = if args.body.is_empty() { "Timer finished" } else { &args.body };
            let packet = proto::notification_packet(proto::AlertCategory::Simple.as_u8(), title, body);
            proto::ensure_notification_packet(&packet).map_err(|error| error.to_string())?;
            writes.push(write_characteristic(link, proto::NEW_ALERT, &packet, args.read_back)?);
            Counts::default()
        }
        "media" => {
            write_media(link, args, &mut writes)?;
            ble::subscribe(link, proto::MUSIC_EVENT).map_err(ble_err)?;
            drain(
                link,
                &device_id,
                &args.wearer,
                args.hold_ms,
                &model,
                &firmware,
                &args.streams,
            )?
        }
        "find" => {
            // Immediate Alert level 2 (high) makes the watch vibrate.
            writes.push(write_characteristic(
                link,
                proto::ALERT_LEVEL,
                &[2],
                args.read_back,
            )?);
            Counts::default()
        }
        "time" => {
            // The watch has no timezone of its own: the caller says which local
            // time to display, and the payload carries exactly those fields.
            let packet = proto::current_time_packet(epoch(args)?, args.offset_min)
                .map_err(|error| error.to_string())?;
            writes.push(write_characteristic(
                link,
                proto::CURRENT_TIME,
                &packet,
                args.read_back,
            )?);
            Counts::default()
        }
        "nav" => {
            let navigation = proto::Navigation {
                icon: args.icon.clone(),
                narrative: args.narrative.clone(),
                distance: args.distance.clone(),
                progress: args.progress,
            };
            for (uuid, bytes) in navigation.packets().map_err(|error| error.to_string())? {
                writes.push(write_characteristic(link, uuid, &bytes, args.read_back)?);
            }
            Counts::default()
        }
        "weather" => {
            let (current, forecast) = if args.from_record {
                weather_from_record(args)?
            } else {
                weather_from_flags(args)?
            };
            writes.push(write_characteristic(
                link,
                proto::WEATHER_DATA,
                &current,
                args.read_back,
            )?);
            if let Some(forecast) = forecast {
                writes.push(write_characteristic(
                    link,
                    proto::WEATHER_DATA,
                    &forecast,
                    args.read_back,
                )?);
            }
            Counts::default()
        }
        "bridge" => {
            // The call-response characteristic is the only inbound channel the
            // bridge needs. Health streams stay off so a decision loop cannot
            // turn into a telemetry firehose.
            ble::subscribe(link, proto::NOTIFICATION_EVENT).map_err(ble_err)?;
            eprintln!("pinetime: subscribed to call responses");
            bridge_loop(link, &args.wearer, args)?
        }
        other => return Err(format!("unknown command {other:?}")),
    };

    let now = time::monotonic_ms().map_err(time_err)?;
    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({
            "ok": true,
            "command": args.command.clone(),
            "device_id": device_id.clone(),
            "model": model.clone(),
            "firmware": firmware.clone(),
            "battery_percent": battery,
            "wearer": args.wearer.clone(),
            "profile": PROFILE.name(),
            "streams": args.streams.iter().map(|stream| stream.as_str()).collect::<Vec<_>>(),
            "signals": counts.json(),
            "bridge": BRIDGE_SUMMARY.with(|slot| slot.borrow().clone()),
            "writes": write_report(&writes),
            "updated_at_monotonic_ms": now,
        });
    });
    Ok(())
}

/// The characteristic a stream comes from, when this watch has one. A stream
/// another profile owns (a ring's SpO2) is absent here rather than invented.
fn characteristic(stream: Stream) -> Option<&'static str> {
    match stream {
        Stream::HeartRate => Some(proto::HEART_RATE_MEASUREMENT),
        Stream::Steps => Some(proto::STEP_COUNT),
        Stream::Motion => Some(proto::MOTION_VALUES),
        Stream::Media => Some(proto::MUSIC_EVENT),
        Stream::Calls => Some(proto::NOTIFICATION_EVENT),
        Stream::Battery => Some(proto::BATTERY_LEVEL),
        Stream::Spo2 => None,
    }
}

/// Subscribe only to the streams the wearer asked for. The health streams are
/// required — without them a watch session has nothing to show — while music,
/// calls and battery are optional, because older firmware may not expose them.
fn subscribe(link: Link, streams: &[Stream]) -> Result<(), String> {
    for stream in streams {
        let Some(uuid) = characteristic(*stream) else {
            continue;
        };
        match ble::subscribe(link, uuid) {
            Ok(()) => {}
            Err(error) if matches!(stream, Stream::HeartRate | Stream::Steps | Stream::Motion) => {
                return Err(ble_err(error));
            }
            Err(error) => println!("pinetime: optional subscribe {uuid} failed: {error:?}"),
        }
    }
    Ok(())
}

fn drain(
    link: Link,
    device_id: &str,
    wearer: &str,
    hold_ms: u64,
    model: &str,
    firmware: &str,
    streams: &[Stream],
) -> Result<Counts, String> {
    let start = time::monotonic_ms().map_err(time_err)?;
    let deadline = start.saturating_add(hold_ms);
    let mut counts = Counts::default();

    loop {
        let now = time::monotonic_ms().map_err(time_err)?;
        if now >= deadline {
            break;
        }
        let wait = (deadline - now).min(1_000) as u32;
        match ble::next_event(link, wait).map_err(ble_err)? {
            Some(Event::Notification(notification)) => {
                handle_notification(
                    device_id,
                    wearer,
                    model,
                    firmware,
                    &notification.char_uuid,
                    &notification.data,
                    streams,
                    &mut counts,
                )?;
            }
            Some(Event::Disconnected(reason)) => {
                return Err(format!("PineTime disconnected: {reason}"));
            }
            None => {}
        }
    }
    Ok(counts)
}

fn handle_notification(
    device_id: &str,
    wearer: &str,
    model: &str,
    firmware: &str,
    char_uuid: &str,
    data: &[u8],
    streams: &[Stream],
    counts: &mut Counts,
) -> Result<(), String> {
    // Battery is device state, not a body signal: it belongs on the device
    // record and never on the signal stream.
    if same(char_uuid, proto::BATTERY_LEVEL) {
        if !streams.contains(&Stream::Battery) {
            return Ok(());
        }
        let battery = proto::parse_battery(data).map_err(|error| error.to_string())?;
        upsert_device_state(device_id, model, firmware, battery, wearer)?;
        counts.bump(Stream::Battery);
        return Ok(());
    }
    let Some(signal) = PROFILE.decode(char_uuid, data)? else {
        return Ok(());
    };
    // `--streams` narrows what the wearer asked for; a notification for a
    // stream that was not selected is not an error, just not wanted.
    if !streams.contains(&signal.stream) {
        return Ok(());
    }
    note_signal(device_id, wearer, &signal)?;
    counts.bump(signal.stream);
    Ok(())
}

/// One characteristic write, and what a read-back said about it. The read
/// happens immediately: two writes to the same characteristic must not be
/// compared against the second value.
struct Written {
    uuid: String,
    bytes: usize,
    /// `None` when `--read-back` was not asked for.
    verified: Option<Verified>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verified {
    Matched,
    Mismatched,
    /// The firmware exposes the characteristic as write-only.
    Unsupported,
}

fn write_characteristic(
    link: Link,
    uuid: &str,
    data: &[u8],
    read_back: bool,
) -> Result<Written, String> {
    ble::write(link, uuid, data).map_err(ble_err)?;
    let verified = if read_back {
        Some(match ble::read(link, uuid, data.len().max(1) as u32) {
            Ok(value) if value == data => Verified::Matched,
            Ok(_) => Verified::Mismatched,
            Err(_) => Verified::Unsupported,
        })
    } else {
        None
    };
    Ok(Written {
        uuid: uuid.to_string(),
        bytes: data.len(),
        verified,
    })
}

fn write_media(link: Link, args: &Args, writes: &mut Vec<Written>) -> Result<(), String> {
    let status = [proto::music_status(args.playing)];
    writes.push(write_characteristic(
        link,
        proto::MUSIC_STATUS,
        &status,
        args.read_back,
    )?);
    for (uuid, value) in [
        (proto::MUSIC_ARTIST, &args.artist),
        (proto::MUSIC_TRACK, &args.track),
        (proto::MUSIC_ALBUM, &args.album),
    ] {
        if !value.is_empty() {
            writes.push(write_characteristic(
                link,
                uuid,
                value.as_bytes(),
                args.read_back,
            )?);
        }
    }
    Ok(())
}

/// The intent a command expresses, for the profile's firmware gate. Commands
/// that use services present since the first release have none.
fn intent_of(command: &str) -> Option<Intent> {
    match command {
        "watch" => Some(Intent::Watch),
        "notify" => Some(Intent::Notify),
        "media" => Some(Intent::Media),
        "find" => Some(Intent::Find),
        "time" => Some(Intent::Time),
        "nav" => Some(Intent::Nav),
        "weather" => Some(Intent::Weather),
        _ => None,
    }
}

/// What went to the radio, and optionally what the watch read back.
fn write_report(writes: &[Written]) -> Vec<Value> {
    writes
        .iter()
        .map(|written| {
            let mut report = json!({"uuid": written.uuid, "bytes": written.bytes});
            if let Some(verified) = written.verified {
                report["read_back"] = match verified {
                    Verified::Matched => json!(true),
                    Verified::Mismatched => json!("mismatch"),
                    Verified::Unsupported => json!("unsupported"),
                };
            }
            report
        })
        .collect()
}

/// The frames the flags path writes: current conditions, then the forecast the
/// caller typed. Every value is the caller's; nothing here reads a record.
fn weather_from_flags(args: &Args) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let icon = proto::WeatherIcon::parse(&args.icon).map_err(|error| error.to_string())?;
    let epoch = epoch(args)?;
    let required = |value: Option<i16>, flag: &str| {
        value
            .ok_or_else(|| format!("weather needs {flag} (or --from-record with a weather record)"))
    };
    let current = proto::current_weather_packet(
        epoch,
        required(args.temp_c, "--temp-c")?,
        required(args.min_c, "--min-c")?,
        required(args.max_c, "--max-c")?,
        &args.location,
        icon,
        args.sunrise_min,
        args.sunset_min,
    );
    let forecast =
        (!args.forecast.is_empty()).then(|| proto::forecast_packet(epoch, &args.forecast));
    Ok((current, forecast))
}

/// The frames the record path writes: the same builders, fed from the
/// `weather.current.v1` record instead of from flags.
fn weather_from_record(args: &Args) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let entry = match &args.record_id {
        Some(id) => records::get(&Key {
            collection: weather::COLLECTION.to_string(),
            id: id.clone(),
        })
        .map_err(|error| format!("weather.current/{id}: {error:?}"))?,
        None => newest_weather_record()?,
    };
    if entry.deleted {
        return Err(format!("weather.current/{} is deleted", entry.key.id));
    }
    if entry.schema != weather::SCHEMA {
        return Err(format!(
            "weather.current/{} is {}, not {}",
            entry.key.id,
            entry.schema,
            weather::SCHEMA
        ));
    }
    let record = weather::CurrentWeather::from_body(&entry.body)
        .map_err(|error| format!("weather.current/{}: {error}", entry.key.id))?;
    weather_frames(&record).map_err(|error| format!("weather.current/{}: {error}", entry.key.id))
}

/// The newest `weather.current.v1` record. The host's revision stamp orders
/// samples across runs; the id breaks ties so two runs agree on the choice.
fn newest_weather_record() -> Result<Entry, String> {
    const PAGE: u32 = 512;
    const MAX_PAGES: usize = 8;
    let mut newest: Option<Entry> = None;
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page = records::scan(weather::COLLECTION, cursor.as_deref(), PAGE)
            .map_err(|error| format!("weather.current: {error:?}"))?;
        for entry in page.entries {
            if entry.deleted || entry.schema != weather::SCHEMA {
                continue;
            }
            let newer = newest.as_ref().is_none_or(|best| {
                (entry.updated_at_ms, &entry.key.id) > (best.updated_at_ms, &best.key.id)
            });
            if newer {
                newest = Some(entry);
            }
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => {
                return newest.ok_or_else(|| {
                    "no weather.current.v1 record: run `weather now` first".to_string()
                })
            }
        }
    }
    Err(format!(
        "weather.current holds more than {} records; pass --id",
        PAGE as usize * MAX_PAGES
    ))
}

/// Record to wire. The frame layout stays in `infinitime`; this only decides
/// which values the watch gets.
fn weather_frames(record: &weather::CurrentWeather) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let timestamp = u64::try_from(record.timestamp).map_err(|_| {
        format!(
            "timestamp {} is before 1970 and does not fit the wire",
            record.timestamp
        )
    })?;
    let icon =
        proto::WeatherIcon::parse(record.icon.as_str()).map_err(|error| error.to_string())?;
    let current = proto::current_weather_packet(
        timestamp,
        hundredths(record.temp_c, "temp_c")?,
        hundredths(record.min_c, "min_c")?,
        hundredths(record.max_c, "max_c")?,
        &record.location,
        icon,
        record.sunrise_min,
        record.sunset_min,
    );
    let forecast = if record.forecast.is_empty() {
        None
    } else {
        let days = record
            .forecast
            .iter()
            .map(|day| {
                Ok(proto::ForecastDay {
                    min_c: hundredths(day.min_c, "forecast.min_c")?,
                    max_c: hundredths(day.max_c, "forecast.max_c")?,
                    icon: proto::WeatherIcon::parse(day.icon.as_str())
                        .map_err(|error| error.to_string())?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Some(proto::forecast_packet(timestamp, &days))
    };
    Ok((current, forecast))
}

/// Degrees Celsius to the wire's hundredths, refusing a value it cannot carry.
fn hundredths(celsius: f64, field: &str) -> Result<i16, String> {
    let hundredths = (celsius * 100.0).round();
    if !(-32_768.0..=32_767.0).contains(&hundredths) {
        return Err(format!("{field}: {celsius} C does not fit the wire"));
    }
    Ok(hundredths as i16)
}

fn epoch(args: &Args) -> Result<u64, String> {
    args.epoch
        .ok_or_else(|| format!("{} needs --epoch <unix-seconds>", args.command))
}

/// Degrees Celsius on the command line, hundredths in the packet.
fn degrees(value: &str, flag: &str) -> Result<i16, String> {
    let parsed: f32 = value
        .parse()
        .map_err(|_| format!("{flag}: {value:?} is not a temperature"))?;
    let hundredths = (parsed * 100.0).round();
    if !(-32_768.0..=32_767.0).contains(&hundredths) {
        return Err(format!("{flag}: {value} is out of range"));
    }
    Ok(hundredths as i16)
}

/// Minutes since midnight, or one of the service's negative sentinels.
fn count(value: &str, flag: &str) -> Result<i16, String> {
    value
        .parse()
        .map_err(|_| format!("{flag}: {value:?} is not a number of minutes"))
}

/// `min,max,icon;min,max,icon` in whole degrees Celsius.
fn forecast(value: &str) -> Result<Vec<proto::ForecastDay>, String> {
    let mut days = Vec::new();
    for entry in value.split(';').filter(|entry| !entry.trim().is_empty()) {
        let mut fields = entry.split(',');
        let min = fields
            .next()
            .ok_or_else(|| format!("--forecast: {entry:?} is not min,max,icon"))?;
        let max = fields
            .next()
            .ok_or_else(|| format!("--forecast: {entry:?} is not min,max,icon"))?;
        let icon = fields
            .next()
            .ok_or_else(|| format!("--forecast: {entry:?} is not min,max,icon"))?;
        if fields.next().is_some() {
            return Err(format!("--forecast: {entry:?} has too many fields"));
        }
        days.push(proto::ForecastDay {
            min_c: degrees(min, "--forecast min")?,
            max_c: degrees(max, "--forecast max")?,
            icon: proto::WeatherIcon::parse(icon).map_err(|error| error.to_string())?,
        });
    }
    Ok(days)
}

fn read_string(link: Link, uuid: &str) -> Result<String, String> {
    let bytes = ble::read(link, uuid, 128).map_err(ble_err)?;
    Ok(String::from_utf8_lossy(&bytes)
        .trim_end_matches('\0')
        .trim()
        .to_string())
}

fn read_battery(link: Link) -> Result<u8, String> {
    let bytes = ble::read(link, proto::BATTERY_LEVEL, 1).map_err(ble_err)?;
    proto::parse_battery(&bytes).map_err(|error| error.to_string())
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
    upsert(
        &watch_core::namespace("device.state", wearer),
        device_id,
        "device.state.v1",
        &body,
    )
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
    upsert(
        &watch_core::namespace("signal", wearer),
        &signal_id,
        "signal.sample.v1",
        &body,
    )
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

fn same(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn ble_err(error: ble::Error) -> String {
    format!("ble: {error:?}")
}

fn time_err(error: time::Error) -> String {
    format!("time: {error:?}")
}

fn set_error(error: &str) {
    SNAPSHOT.with(|slot| {
        *slot.borrow_mut() = json!({"ok": false, "error": error});
    });
}

const JARVIS_DECISION: &str = "jarvis.decision.v1";
const JARVIS_NOTICE: &str = "jarvis.notice.v1";
const JARVIS_REPLY: &str = "jarvis.reply.v1";
const JARVIS_RESOLVE: &str = "jarvis.decision.resolve.v1";
const JARVIS_DELIVERY: &str = "jarvis.delivery.v1";
const BRIDGE_KINDS: [&str; 4] = [JARVIS_DECISION, JARVIS_NOTICE, JARVIS_REPLY, JARVIS_RESOLVE];
const BRIDGE_QUEUE_MAX: usize = 8;
const BRIDGE_SEEN_MAX: usize = 256;
const BRIDGE_TTL_MS: u64 = 60_000;

fn bridge_namespace(wearer: &str) -> String {
    watch_core::namespace("bridge", wearer)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveDecision {
    decision: Decision,
    /// Monotonic receipt time, used to age the frame's wall-clock stamp.
    received_mono_ms: u64,
    /// The bus frame's wall-clock stamp when it was published.
    issued_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WallAnchor {
    mono_ms: u64,
    wall_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct BridgeRuntime {
    /// Content hashes already applied. Live Unibus drains; the dev MemoryBus
    /// retains frames, so this keeps a poll loop idempotent on both.
    seen: Vec<String>,
    active: Option<ActiveDecision>,
    queue: std::collections::VecDeque<ActiveDecision>,
    anchor: Option<WallAnchor>,
}

fn bridge_loop(link: Link, wearer: &str, args: &Args) -> Result<Counts, String> {
    let mut runtime = load_bridge_state(wearer);
    let mut counts = Counts::default();
    let start = time::monotonic_ms().map_err(time_err)?;
    // Monotonic values are process-local. Reset the persisted wall anchor and
    // receipt time so a restored item is not expired across two epochs.
    runtime.anchor = None;
    if let Some(active) = &mut runtime.active {
        active.received_mono_ms = start;
    }
    let person = args.person.as_deref().unwrap_or(wearer);
    let poll_ms = u64::from(args.poll_ms.max(50));
    let deadline = if args.hold_ms == 0 {
        u64::MAX
    } else {
        start.saturating_add(args.hold_ms)
    };
    let mut next_poll = start;

    let restored_wall = runtime
        .active
        .as_ref()
        .map_or(start, |active| active.issued_at_ms);
    if let Some(active) = runtime.active.clone() {
        show_decision(link, &active)?;
        publish_delivery(&active.decision.id, "shown", restored_wall)?;
        counts.bump_key("decisions_shown");
    }
    show_next(link, &mut runtime, &mut counts, restored_wall)?;
    save_bridge_state(wearer, &runtime)?;

    loop {
        let now = time::monotonic_ms().map_err(time_err)?;
        if now >= deadline {
            break;
        }
        if now >= next_poll {
            poll_bridge(link, &mut runtime, wearer, person, &mut counts)?;
            next_poll = now.saturating_add(poll_ms);
        }

        let wall = wall_now(&runtime, now);
        if runtime.anchor.is_some() {
            if let Some(active) = runtime.active.clone() {
                if is_expired(&active, wall) {
                    publish_delivery(&active.decision.id, "expired", wall)?;
                    runtime.active = None;
                    save_bridge_state(wearer, &runtime)?;
                }
            }
        }
        show_next(link, &mut runtime, &mut counts, wall)?;

        let wait = deadline.saturating_sub(now).min(poll_ms).clamp(1, 1_000) as u32;
        match ble::next_event(link, wait).map_err(ble_err)? {
            Some(Event::Notification(notification)) => {
                eprintln!(
                    "pinetime: notification {} {} byte(s)",
                    notification.char_uuid,
                    notification.data.len()
                );
                if same(&notification.char_uuid, proto::NOTIFICATION_EVENT) {
                    handle_tap(link, &mut runtime, wearer, &notification.data, &mut counts)?;
                }
            }
            Some(Event::Disconnected(reason)) => {
                return Err(format!("PineTime disconnected: {reason}"));
            }
            None => {}
        }
    }

    save_bridge_state(wearer, &runtime)?;
    BRIDGE_SUMMARY.with(|slot| {
        *slot.borrow_mut() = json!({
            "active_decision_id": runtime.active.as_ref().map(|active| active.decision.id.clone()),
            "queued": runtime.queue.len(),
            "seen": runtime.seen.len(),
        });
    });
    Ok(counts)
}

fn poll_bridge(
    link: Link,
    runtime: &mut BridgeRuntime,
    wearer: &str,
    person: &str,
    counts: &mut Counts,
) -> Result<(), String> {
    let kinds = BRIDGE_KINDS
        .iter()
        .map(|kind| (*kind).to_string())
        .collect::<Vec<_>>();
    let page = bus::poll(&kinds, None, 32).map_err(|error| format!("unibus poll: {error:?}"))?;
    let now = time::monotonic_ms().map_err(time_err)?;
    let mut wall = wall_now(runtime, now);
    let mut changed = false;

    for frame in page.frames {
        let key = frame_key(&frame.kind, &frame.body);
        if runtime.seen.iter().any(|seen| seen == &key) {
            continue;
        }
        changed = true;
        runtime.seen.push(key);
        while runtime.seen.len() > BRIDGE_SEEN_MAX {
            runtime.seen.remove(0);
        }

        if runtime.anchor.is_none() && frame.issued_at_ms > 0 {
            runtime.anchor = Some(WallAnchor {
                mono_ms: now,
                wall_ms: frame.issued_at_ms,
            });
            wall = frame.issued_at_ms;
        }

        let Ok(value) = serde_json::from_slice::<Value>(&frame.body) else {
            eprintln!("pinetime: ignoring malformed {} body", frame.kind);
            continue;
        };

        match frame.kind.as_str() {
            JARVIS_DECISION => {
                let Some(decision) = bridge::parse_decision(&value) else {
                    continue;
                };
                if !person_matches(decision.person.as_deref(), person) {
                    continue;
                }
                let active = ActiveDecision {
                    decision,
                    received_mono_ms: now,
                    issued_at_ms: frame.issued_at_ms,
                };
                if runtime.queue.len() >= BRIDGE_QUEUE_MAX {
                    publish_delivery(&active.decision.id, "dropped", wall)?;
                } else {
                    runtime.queue.push_back(active);
                }
            }
            JARVIS_NOTICE => {
                let Some(notice) = bridge::parse_notice(&value) else {
                    continue;
                };
                if !person_matches(notice.person.as_deref(), person) {
                    continue;
                }
                if notice
                    .expires_at_ms
                    .is_some_and(|expires| expires > 0 && wall >= expires)
                {
                    publish_delivery(&notice.id, "expired", wall)?;
                    continue;
                }
                match show_notice(link, &notice) {
                    Ok(()) => {
                        publish_delivery(&notice.id, "shown", wall)?;
                        counts.bump_key("notices_shown");
                    }
                    Err(error) => {
                        eprintln!("pinetime: notice {} not shown: {error}", notice.id);
                        publish_delivery(&notice.id, "failed", wall)?;
                    }
                }
            }
            JARVIS_REPLY => {
                let Some(reply) = bridge::parse_reply(&value) else {
                    continue;
                };
                if !reply.state.eq_ignore_ascii_case("final") {
                    continue;
                }
                if let Some(surface) = reply.surface.as_deref() {
                    if !surface.eq_ignore_ascii_case("watch")
                        && !surface.eq_ignore_ascii_case("all")
                    {
                        continue;
                    }
                }
                if !person_matches(reply.person.as_deref(), person) {
                    continue;
                }
                let notice = bridge::Notice {
                    id: reply.id,
                    title: "JARVIS".to_string(),
                    body: reply.text,
                    person: reply.person,
                    created_at_ms: reply.at_ms,
                    expires_at_ms: None,
                };
                match show_notice(link, &notice) {
                    Ok(()) => {
                        publish_delivery(&notice.id, "shown", wall)?;
                        counts.bump_key("notices_shown");
                    }
                    Err(error) => {
                        eprintln!("pinetime: reply {} not shown: {error}", notice.id);
                        publish_delivery(&notice.id, "failed", wall)?;
                    }
                }
            }
            JARVIS_RESOLVE => {
                let Some(resolution) = bridge::parse_resolution(&value) else {
                    continue;
                };
                if resolution.surface.eq_ignore_ascii_case("watch") {
                    continue;
                }
                if runtime
                    .active
                    .as_ref()
                    .is_some_and(|active| active.decision.id == resolution.decision_id)
                {
                    publish_delivery(&resolution.decision_id, "resolved", wall)?;
                    runtime.active = None;
                } else {
                    runtime
                        .queue
                        .retain(|active| active.decision.id != resolution.decision_id);
                }
            }
            _ => {}
        }
    }

    show_next(link, runtime, counts, wall)?;
    if changed {
        save_bridge_state(wearer, runtime)?;
    }
    Ok(())
}

fn handle_tap(
    link: Link,
    runtime: &mut BridgeRuntime,
    wearer: &str,
    data: &[u8],
    counts: &mut Counts,
) -> Result<(), String> {
    let Some(byte) = data.first().copied() else {
        return Ok(());
    };
    let Some(response) = WatchResponse::from_byte(byte) else {
        eprintln!("pinetime: unknown call response 0x{byte:02x}");
        return Ok(());
    };
    eprintln!("pinetime: call response {response:?}");
    let Some(active) = runtime.active.clone() else {
        return Ok(());
    };

    // Only an approval decision has a call screen with these buttons. A
    // response byte can also come from a real incoming call, or from a
    // notification the user dismissed; do not let those clear a plain alert.
    if !bridge::is_approval(&active.decision.options) {
        eprintln!(
            "pinetime: {} is a plain alert; ignoring {response:?}",
            active.decision.id
        );
        return Ok(());
    }

    let now = time::monotonic_ms().map_err(time_err)?;
    let wall = wall_now(runtime, now);
    if runtime.anchor.is_some() && is_expired(&active, wall) {
        publish_delivery(&active.decision.id, "expired", wall)?;
        runtime.active = None;
        show_next(link, runtime, counts, wall)?;
        save_bridge_state(wearer, runtime)?;
        return Ok(());
    }
    match response {
        WatchResponse::Mute => {
            publish_delivery(&active.decision.id, "muted", wall)?;
        }
        WatchResponse::Accept | WatchResponse::Reject => {
            let Some(answer) = bridge::answer_for(response, &active.decision.options) else {
                return Ok(());
            };
            let body = bridge::resolution(&active.decision.id, &answer, wearer, wall);
            publish_json(JARVIS_RESOLVE, &body)?;
            publish_delivery(&active.decision.id, "resolved", wall)?;
            eprintln!("pinetime: resolved {} as {answer}", active.decision.id);
            if let Err(error) = show_resolution_notice(link, response, &active.decision) {
                eprintln!("pinetime: resolution notice failed: {error}");
            }
            counts.bump_key("resolutions");
        }
    }

    runtime.active = None;
    show_next(link, runtime, counts, wall)?;
    save_bridge_state(wearer, runtime)?;
    Ok(())
}

fn show_next(
    link: Link,
    runtime: &mut BridgeRuntime,
    counts: &mut Counts,
    wall: u64,
) -> Result<(), String> {
    while runtime.active.is_none() {
        let Some(active) = runtime.queue.pop_front() else {
            break;
        };
        if is_expired(&active, wall) {
            publish_delivery(&active.decision.id, "expired", wall)?;
            continue;
        }
        match show_decision(link, &active) {
            Ok(()) => {
                publish_delivery(&active.decision.id, "shown", wall)?;
                counts.bump_key("decisions_shown");
                runtime.active = Some(active);
            }
            Err(error) => {
                eprintln!(
                    "pinetime: decision {} not shown: {error}",
                    active.decision.id
                );
                publish_delivery(&active.decision.id, "failed", wall)?;
            }
        }
    }
    Ok(())
}

fn show_decision(link: Link, active: &ActiveDecision) -> Result<(), String> {
    let (category, kind) = if bridge::is_approval(&active.decision.options) {
        (
            proto::AlertCategory::Call.as_u8(),
            proto::DisplayKind::Approval,
        )
    } else {
        (
            proto::AlertCategory::Simple.as_u8(),
            proto::DisplayKind::Plain,
        )
    };
    let packet = proto::notification_packet_with_kind(
        category,
        kind,
        &active.decision.summary,
        &active.decision.detail,
    );
    ble::write(link, proto::NEW_ALERT, &packet).map_err(ble_err)
}

fn show_notice(link: Link, notice: &bridge::Notice) -> Result<(), String> {
    let packet = proto::notification_packet_with_kind(
        proto::AlertCategory::Simple.as_u8(),
        proto::DisplayKind::Plain,
        &notice.title,
        &notice.body,
    );
    ble::write(link, proto::NEW_ALERT, &packet).map_err(ble_err)
}

fn show_resolution_notice(
    link: Link,
    response: WatchResponse,
    decision: &Decision,
) -> Result<(), String> {
    let (title, body) = match response {
        WatchResponse::Accept => ("Approved", decision.summary.as_str()),
        WatchResponse::Reject => ("Declined", decision.summary.as_str()),
        WatchResponse::Mute => ("Muted", "Decision left open"),
    };
    let packet = proto::notification_packet_with_kind(
        proto::AlertCategory::Simple.as_u8(),
        proto::DisplayKind::Plain,
        title,
        body,
    );
    ble::write(link, proto::NEW_ALERT, &packet).map_err(ble_err)
}

fn publish_delivery(item_id: &str, state: &str, at_ms: u64) -> Result<(), String> {
    publish_json(JARVIS_DELIVERY, &bridge::delivery(item_id, state, at_ms))
}

fn publish_json(kind: &str, body: &Value) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(body).map_err(|error| format!("publish {kind} JSON: {error}"))?;
    bus::publish(kind, &bytes, None, BRIDGE_TTL_MS)
        .map(|_| ())
        .map_err(|error| format!("unibus publish {kind}: {error:?}"))
}

fn wall_now(runtime: &BridgeRuntime, now_mono: u64) -> u64 {
    if let Some(anchor) = &runtime.anchor {
        return anchor
            .wall_ms
            .saturating_add(now_mono.saturating_sub(anchor.mono_ms));
    }
    if let Some(active) = &runtime.active {
        return active
            .issued_at_ms
            .saturating_add(now_mono.saturating_sub(active.received_mono_ms));
    }
    now_mono
}

fn is_expired(active: &ActiveDecision, wall_ms: u64) -> bool {
    active
        .decision
        .expires_at_ms
        .is_some_and(|expires| expires > 0 && wall_ms >= expires)
}

fn person_matches(person: Option<&str>, wanted: &str) -> bool {
    person.is_none_or(|person| person.eq_ignore_ascii_case(wanted))
}

fn frame_key(kind: &str, body: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{kind}:{hash:016x}")
}

fn load_bridge_state(wearer: &str) -> BridgeRuntime {
    let mut runtime = records::get(&Key {
        collection: bridge_namespace(wearer),
        id: "jarvis.state".to_string(),
    })
    .ok()
    .and_then(|entry| serde_json::from_slice::<BridgeRuntime>(&entry.body).ok())
    .unwrap_or_default();
    runtime.anchor = None;
    runtime
}

fn save_bridge_state(wearer: &str, runtime: &BridgeRuntime) -> Result<(), String> {
    let value =
        serde_json::to_value(runtime).map_err(|error| format!("bridge state JSON: {error}"))?;
    upsert(
        &bridge_namespace(wearer),
        "jarvis.state",
        "jarvis.state.v1",
        &value,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `weather.current.v1` body as the weather Spore writes it: Seattle,
    /// 2026-11-13T20:45 local, five days.
    fn record_body() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "location": "Seattle",
            "latitude": 47.6062,
            "longitude": -122.3321,
            "timestamp": 1_794_602_700i64,
            "utc_offset_seconds": -28_800,
            "timezone": "America/Los_Angeles",
            "temp_c": 12.8,
            "min_c": 8.1,
            "max_c": 14.2,
            "icon": "rain",
            "sunrise_min": 432,
            "sunset_min": 998,
            "source": "open-meteo",
            "model_version": "weather-spore@0.1.0",
            "fetched_at_monotonic_ms": 5_000,
            "forecast": [
                {"date": "2026-11-13", "min_c": 8.1, "max_c": 14.2, "icon": "rain"},
                {"date": "2026-11-14", "min_c": 7.4, "max_c": 13.1, "icon": "clouds"},
                {"date": "2026-11-15", "min_c": 5.2, "max_c": 11.0, "icon": "clouds-rain"},
                {"date": "2026-11-16", "min_c": -1.0, "max_c": 4.5, "icon": "snow"},
                {"date": "2026-11-17", "min_c": 0.5, "max_c": 6.0, "icon": "clear"},
            ],
        })
    }

    fn record() -> weather::CurrentWeather {
        let body = serde_json::to_vec(&record_body()).expect("body");
        weather::CurrentWeather::from_body(&body).expect("record")
    }

    #[test]
    fn a_weather_record_becomes_the_53_byte_frame() {
        let (current, _) = weather_frames(&record()).expect("frames");

        let mut expected = vec![0u8, 1];
        expected.extend_from_slice(&1_794_602_700u64.to_le_bytes());
        expected.extend_from_slice(&1280i16.to_le_bytes());
        expected.extend_from_slice(&810i16.to_le_bytes());
        expected.extend_from_slice(&1420i16.to_le_bytes());
        let mut location = [0u8; 32];
        location[..7].copy_from_slice(b"Seattle");
        expected.extend_from_slice(&location);
        expected.push(5); // rain
        expected.extend_from_slice(&432i16.to_le_bytes());
        expected.extend_from_slice(&998i16.to_le_bytes());

        assert_eq!(current.len(), 53);
        assert_eq!(current, expected);
    }

    #[test]
    fn a_weather_record_becomes_the_36_byte_forecast_frame() {
        let (_, forecast) = weather_frames(&record()).expect("frames");
        let forecast = forecast.expect("the record carries a forecast");

        let mut expected = vec![1u8, 0];
        expected.extend_from_slice(&1_794_602_700u64.to_le_bytes());
        expected.push(5);
        for (min, max, icon) in [
            (810i16, 1420i16, 5u8),
            (740, 1310, 2),
            (520, 1100, 4),
            (-100, 450, 7),
            (50, 600, 0),
        ] {
            expected.extend_from_slice(&min.to_le_bytes());
            expected.extend_from_slice(&max.to_le_bytes());
            expected.push(icon);
        }

        assert_eq!(forecast.len(), 36);
        assert_eq!(forecast, expected);
    }

    #[test]
    fn a_record_without_forecast_writes_no_forecast_frame() {
        let mut body = record_body();
        body["forecast"] = json!([]);
        let record = weather::CurrentWeather::from_body(&serde_json::to_vec(&body).unwrap())
            .expect("record");
        let (current, forecast) = weather_frames(&record).expect("frames");
        assert_eq!(current.len(), 53);
        assert_eq!(forecast, None);
    }

    #[test]
    fn a_polar_record_keeps_the_absent_sentinel() {
        let mut body = record_body();
        body["sunrise_min"] = json!(-2);
        body["sunset_min"] = json!(-2);
        let record = weather::CurrentWeather::from_body(&serde_json::to_vec(&body).unwrap())
            .expect("record");
        let (current, _) = weather_frames(&record).expect("frames");
        assert_eq!(&current[49..51], &(-2i16).to_le_bytes());
        assert_eq!(&current[51..53], &(-2i16).to_le_bytes());
        assert_ne!(&current[49..51], &0i16.to_le_bytes());
    }

    #[test]
    fn the_record_and_the_wire_agree_on_the_sun_sentinels() {
        assert_eq!(weather::SUN_ABSENT, proto::SUN_ABSENT);
        assert_eq!(proto::SUN_UNKNOWN, -1);
    }

    #[test]
    fn every_record_icon_name_is_a_wire_icon() {
        for icon in weather::Icon::ALL {
            assert!(
                proto::WeatherIcon::parse(icon.as_str()).is_ok(),
                "the wire does not know {:?}",
                icon.as_str()
            );
        }
        // The one icon the provider never selects still has a wire id.
        assert_eq!(
            proto::WeatherIcon::parse("heavy-clouds"),
            Ok(proto::WeatherIcon::HeavyClouds)
        );
    }

    #[test]
    fn a_record_with_an_unknown_icon_never_reaches_the_wire() {
        // The name-to-icon table is the record crate's, so the refusal happens
        // when the body is read rather than during the frame build.
        let mut body = record_body();
        body["icon"] = json!("hail");
        let error =
            weather::CurrentWeather::from_body(&serde_json::to_vec(&body).unwrap()).unwrap_err();
        assert_eq!(error.to_string(), "unknown weather icon \"hail\"");
    }

    #[test]
    fn a_timestamp_before_1970_is_refused() {
        let mut body = record_body();
        body["timestamp"] = json!(-1);
        let record = weather::CurrentWeather::from_body(&serde_json::to_vec(&body).unwrap())
            .expect("record");
        let error = weather_frames(&record).unwrap_err();
        assert!(error.contains("before 1970"), "{error}");
    }

    #[test]
    fn a_temperature_that_does_not_fit_is_refused() {
        let mut body = record_body();
        body["temp_c"] = json!(400.0);
        let record = weather::CurrentWeather::from_body(&serde_json::to_vec(&body).unwrap())
            .expect("record");
        let error = weather_frames(&record).unwrap_err();
        assert_eq!(error, "temp_c: 400 C does not fit the wire");
    }

    fn argv(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn from_record_takes_no_weather_flags() {
        let error = Args::parse(&argv("weather --from-record --epoch 1")).unwrap_err();
        assert_eq!(
            error,
            "--epoch conflicts with --from-record; the record carries every weather field"
        );
    }

    #[test]
    fn an_id_needs_from_record() {
        let error = Args::parse(&argv("weather --id seattle")).unwrap_err();
        assert_eq!(error, "--id needs --from-record");
    }

    #[test]
    fn from_record_only_applies_to_weather() {
        let error = Args::parse(&argv("notify --from-record")).unwrap_err();
        assert_eq!(error, "--from-record only applies to weather");
    }

    #[test]
    fn the_flags_path_still_parses_its_own_values() {
        let args =
            Args::parse(&argv("weather --from-record --id seattle --read-back")).expect("args");
        assert!(args.from_record);
        assert_eq!(args.record_id.as_deref(), Some("seattle"));

        let args = Args::parse(&argv(
            "weather --epoch 1794602700 --icon rain --temp-c 12.8 --min-c 8.1 --max-c 14.2 --location Seattle",
        ))
        .expect("args");
        assert!(!args.from_record);
        let (current, forecast) = weather_from_flags(&args).expect("frames");
        assert_eq!(current.len(), 53);
        assert_eq!(forecast, None);
    }

    #[test]
    fn the_flags_path_refuses_a_missing_temperature() {
        let args = Args::parse(&argv(
            "weather --epoch 1794602700 --icon rain --temp-c 12.8 --max-c 14.2",
        ))
        .expect("args");
        let error = weather_from_flags(&args).unwrap_err();
        assert_eq!(
            error,
            "weather needs --min-c (or --from-record with a weather record)"
        );
    }
}
