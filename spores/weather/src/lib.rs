//! The weather Spore: "what is the weather here", as a record the watch reads.
//!
//! Three providers — Open-Meteo (no key), NWS (US points), METAR (station
//! observations) — one `weather.current` record per place, and one line of
//! output for a person. The Spore owns argument parsing, the
//! place lookup, and the record; the host owns the connection, the origin
//! allowlist, and durability. Nothing here opens a socket or a file.
//!
//! The record is the product. `pinetime weather --from-record` reads it back,
//! so the same sample reaches the watch without a second hand-typed copy of
//! the numbers.

#[cfg(target_arch = "wasm32")]
mod guest;

use serde_json::{json, Value};
use weather_protocol as weather;

/// The host capabilities this Spore needs, in plain Rust so the command logic
/// is host-testable without a wasm runtime.
pub trait Host {
    /// One bounded, allowlisted GET. The host decides which origins exist.
    fn get(&mut self, url: &str) -> Result<Vec<u8>, String>;
    /// Write the record. `id` is stable within [`weather::COLLECTION`].
    fn upsert(&mut self, id: &str, body: &[u8]) -> Result<(), String>;
    /// The host's monotonic clock, for `fetched_at_monotonic_ms`.
    fn monotonic_ms(&mut self) -> Result<u64, String>;
}

/// What one successful run produced: the line to print, and the snapshot a
/// host can show without re-reading the store.
pub struct Outcome {
    pub output: String,
    pub snapshot: Value,
}

/// Run one command and return its output.
pub fn run(args: &[String], host: &mut dyn Host) -> Result<String, String> {
    execute(args, host).map(|outcome| outcome.output)
}

pub fn execute(args: &[String], host: &mut dyn Host) -> Result<Outcome, String> {
    Command::parse(args)?.execute(host)
}

struct Command {
    place: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    label: Option<String>,
    json: bool,
    help: bool,
    provider: String,
    /// METAR station id (`KSEA`). Only `--provider metar` uses it.
    station: Option<String>,
    /// The viewer's offset from UTC, for a METAR observation: the report is
    /// always UTC, and the record carries the shift so the watch shows a
    /// local time rather than a five-hour lie.
    utc_offset_min: i32,
}

impl Command {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut command = Self {
            place: None,
            latitude: None,
            longitude: None,
            label: None,
            json: false,
            help: false,
            provider: "open-meteo".to_string(),
            station: None,
            utc_offset_min: 0,
        };
        let mut words = Vec::new();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--place" => command.place = Some(take(args, &mut index, "--place")?),
                "--lat" => {
                    let value = take(args, &mut index, "--lat")?;
                    command.latitude = Some(number(&value, "--lat")?);
                }
                "--lon" => {
                    let value = take(args, &mut index, "--lon")?;
                    command.longitude = Some(number(&value, "--lon")?);
                }
                "--label" => command.label = Some(take(args, &mut index, "--label")?),
                "--json" => command.json = true,
                "--provider" => {
                    let value = take(args, &mut index, "--provider")?;
                    if value != "open-meteo" && value != "nws" && value != "metar" {
                        return Err(format!(
                            "--provider: {value:?} is not one of open-meteo, nws, metar"
                        ));
                    }
                    command.provider = value;
                }
                "--station" => command.station = Some(take(args, &mut index, "--station")?),
                "--utc-offset-min" => {
                    let value = take(args, &mut index, "--utc-offset-min")?;
                    command.utc_offset_min = value
                        .parse::<i32>()
                        .map_err(|_| format!("--utc-offset-min: {value:?} is not a number"))?;
                }
                "--help" | "-h" | "help" => command.help = true,
                flag if flag.starts_with("--") => return Err(format!("unknown argument {flag:?}")),
                word => words.push(word.to_string()),
            }
            index += 1;
        }
        if command.help {
            return Ok(command);
        }
        match words.as_slice() {
            [verb] if verb == "now" => Ok(command),
            [] => Err(format!(
                "weather needs a command; try `weather now --place Seattle`\n\n{HELP}"
            )),
            [verb, ..] => Err(format!("unknown command {verb:?}; expected now")),
        }
    }

    fn execute(self, host: &mut dyn Host) -> Result<Outcome, String> {
        if self.help {
            return Ok(Outcome {
                output: HELP.to_string(),
                snapshot: json!({"ok": true, "help": true}),
            });
        }
        let json = self.json;
        let provider = self.provider.clone();

        // METAR is station-shaped: the station's own coordinates come back with
        // the observation, so there is nothing to geocode.
        if provider == "metar" {
            let station = self.station.clone().ok_or_else(|| {
                "--provider metar needs --station, e.g. --station KSEA".to_string()
            })?;
            let fetched_at_monotonic_ms = host
                .monotonic_ms()
                .map_err(|error| format!("time: {error}"))?;
            let body = fetch(host, &weather::metar::station_url(&station), "metar")?;
            let record = weather::metar::parse(
                &body,
                &station,
                self.label.as_deref(),
                fetched_at_monotonic_ms,
                self.utc_offset_min,
            )
            .map_err(|error| error.to_string())?
            .with_computed_sun();
            weather::check_location(&record.location)
                .map_err(|error| format!("{error}; pass --label with a shorter name"))?;
            let id = weather::record_id(Some(&station), record.latitude, record.longitude)
                .map_err(|error| error.to_string())?;
            return finish(host, record, id, json);
        }

        let (latitude, longitude, label, id_label) = self.resolve(host)?;
        if !(-90.0..=90.0).contains(&latitude) {
            return Err(format!("--lat: {latitude} is outside -90..90"));
        }
        if !(-180.0..=180.0).contains(&longitude) {
            return Err(format!("--lon: {longitude} is outside -180..180"));
        }
        weather::check_location(&label)
            .map_err(|error| format!("{error}; pass --label with a shorter name"))?;
        let id = weather::record_id(id_label.as_deref(), latitude, longitude)
            .map_err(|error| error.to_string())?;

        let fetched_at_monotonic_ms = host
            .monotonic_ms()
            .map_err(|error| format!("time: {error}"))?;
        let request = weather::SampleRequest {
            location: &label,
            latitude,
            longitude,
            fetched_at_monotonic_ms,
        };
        let record = match provider.as_str() {
            "nws" => {
                // Two calls: the point resolves a grid, the grid's forecast URL
                // returns the periods.
                let points = fetch(
                    host,
                    &weather::nws::points_url(weather::nws::ORIGIN, latitude, longitude),
                    "nws points",
                )?;
                let grid = weather::nws::parse_points(&points)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| {
                        "nws: this point is outside the National Weather Service's coverage; use --provider open-meteo".to_string()
                    })?;
                let body = fetch(host, &grid.forecast_url, "nws forecast")?;
                weather::nws::parse_forecast(&body, &request, &grid)
                    .map_err(|error| error.to_string())?
            }
            _ => {
                let url = weather::forecast_url(weather::FORECAST_ORIGIN, latitude, longitude);
                let body = fetch(host, &url, "open-meteo")?;
                weather::parse_forecast(&body, &request).map_err(|error| error.to_string())?
            }
        }
        .with_computed_sun();

        finish(host, record, id, json)
    }

    /// Turn the flags into a point and the two names a record needs: the string
    /// the watch shows, and the one the record id is derived from (`None` for a
    /// bare coordinate query, whose id is the coordinates themselves).
    fn resolve(self, host: &mut dyn Host) -> Result<(f64, f64, String, Option<String>), String> {
        match (self.place, self.latitude, self.longitude) {
            (Some(place), None, None) => {
                let url = weather::geocode_url(weather::GEOCODING_ORIGIN, &place);
                let body = fetch(host, &url, "geocoding")?;
                let found = weather::parse_geocode(&body)
                    .map_err(|error| format!("geocoding: {error}"))?
                    .ok_or_else(|| format!("geocoding: no place matched {place:?}"))?;
                let label = self.label.unwrap_or(found.name);
                Ok((found.latitude, found.longitude, label.clone(), Some(label)))
            }
            (None, Some(latitude), Some(longitude)) => {
                // No label means the record id is the point itself, not a slug
                // of one: the coordinates are already stable and readable.
                let id_label = self.label.clone();
                let label = self
                    .label
                    .unwrap_or_else(|| weather::coordinates(latitude, longitude));
                Ok((latitude, longitude, label, id_label))
            }
            (Some(_), _, _) => Err("use --place, or --lat and --lon, not both".to_string()),
            (None, None, None) => {
                Err("weather now needs a place: pass --place NAME, or --lat and --lon".to_string())
            }
            (None, Some(_), None) => Err("--lat needs --lon as well".to_string()),
            (None, None, Some(_)) => Err("--lon needs --lat as well".to_string()),
        }
    }
}

/// Write the record and turn it into the line (or the JSON) the caller sees.
fn finish(
    host: &mut dyn Host,
    record: weather::CurrentWeather,
    id: String,
    json: bool,
) -> Result<Outcome, String> {
    let record_body = record.body();
    let bytes =
        serde_json::to_vec(&record_body).map_err(|error| format!("record JSON: {error}"))?;
    host.upsert(&id, &bytes)
        .map_err(|error| format!("records: {}/{id}: {error}", weather::COLLECTION))?;

    let output = if json {
        serde_json::to_string_pretty(&record_body)
            .map_err(|error| format!("record JSON: {error}"))?
    } else {
        summary(&record, &id)
    };
    Ok(Outcome {
        output,
        snapshot: json!({
            "ok": true,
            "collection": weather::COLLECTION,
            "schema": weather::SCHEMA,
            "id": id,
            "location": record.location,
            "timestamp": record.timestamp,
            "fetched_at_monotonic_ms": record.fetched_at_monotonic_ms,
        }),
    })
}

fn take(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn number(value: &str, flag: &str) -> Result<f64, String> {
    value
        .parse()
        .map_err(|_| format!("{flag}: {value:?} is not a number"))
}

fn fetch(host: &mut dyn Host, url: &str, what: &str) -> Result<Vec<u8>, String> {
    host.get(url)
        .map_err(|error| format!("{what}: {error} ({url})"))
}

/// The one line a person reads: what it is like, and where it was written.
fn summary(record: &weather::CurrentWeather, id: &str) -> String {
    let sun = if record.sunrise_min < 0 && record.sunset_min < 0 {
        "no sunrise or sunset today".to_string()
    } else {
        format!(
            "sunrise {}, sunset {}",
            clock(record.sunrise_min),
            clock(record.sunset_min)
        )
    };
    // A station that reported no six-hour extremes has the same number in min
    // and max; printing that as a range would be a lie dressed as data.
    let range = if record.range_reported {
        format!("{:.1}\u{b0}C to {:.1}\u{b0}C", record.min_c, record.max_c)
    } else {
        "no six-hour range reported".to_string()
    };
    format!(
        "{}: {:.1}\u{b0}C {}, {range}, {sun} [{}/{}]",
        record.location,
        record.temp_c,
        record.icon.as_str().replace('-', " "),
        weather::COLLECTION,
        id,
    )
}

fn clock(minutes: i16) -> String {
    if minutes < 0 {
        return "none".to_string();
    }
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

const HELP: &str = "\
usage: weather now --place NAME [--label LABEL] [--json]
       weather now --lat DEGREES --lon DEGREES [--label LABEL] [--json]
       weather now --provider nws --lat DEGREES --lon DEGREES
       weather now --provider metar --station KSEA [--utc-offset-min -480]

commands:
  now                 fetch the current weather, write the record, print a line

providers:
  open-meteo          any point on earth, no key (default)
  nws                 US points, grid forecast
  metar               an airport station: an observation, not a forecast,
                      with the station's own six-hour min/max

examples:
  weather now --place \"Seattle\"
  weather now --lat 47.6 --lon -122.3 --label \"Living room\"
  weather now --provider metar --station KSEA --utc-offset-min -480
  weather now --place Seattle --json";

#[cfg(test)]
mod tests;
