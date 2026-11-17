//! METAR observations from the Aviation Weather Center.
//!
//! Source: <https://aviationweather.gov/data/api/> — `/api/data/metar?ids=…&format=json`
//! returns one object per report, with `obsTime` (unix seconds), `temp`,
//! `minT`/`maxT` (the station's six-hour extremes), `cover`, and the raw METAR.
//!
//! A METAR is an **observation**, not a forecast: `temp_c` is what the station
//! measured, `min_c`/`max_c` are the extremes it reported for the last six
//! hours, and the forecast list is empty. Stating that plainly matters more
//! than filling a daily range we were never given.

use serde_json::Value;

use crate::{CurrentWeather, Error, Icon, SUN_UNKNOWN};

pub const ORIGIN: &str = "https://aviationweather.gov";

/// The observation for one station, the last two hours (one is enough; two
/// tolerates a station that just missed a report).
#[must_use]
pub fn station_url(station: &str) -> String {
    format!("{ORIGIN}/api/data/metar?ids={station}&format=json&hours=2")
}

/// Parse the first report for `station` into the record shape.
///
/// `utc_offset_min` is the viewer's offset from UTC: METAR is always UTC, and
/// the record carries the offset it was shifted by so the watch shows a local
/// time instead of a five-hour lie.
pub fn parse(
    body: &[u8],
    station: &str,
    label: Option<&str>,
    fetched_at_monotonic_ms: u64,
    utc_offset_min: i32,
) -> Result<CurrentWeather, Error> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| Error::Json(error.to_string()))?;
    let reports = value
        .as_array()
        .ok_or_else(|| Error::Json("METAR response is not an array".to_string()))?;
    let report = reports
        .iter()
        .find(|report| {
            report["icaoId"]
                .as_str()
                .is_some_and(|id| id.eq_ignore_ascii_case(station))
        })
        .or_else(|| reports.first())
        .ok_or_else(|| Error::Json(format!("no METAR for {station}")))?;

    let temp_c = report["temp"]
        .as_f64()
        .ok_or_else(|| Error::Json(format!("{station}: METAR has no temperature")))?;
    // Six-hour extremes when the station reports them; otherwise the sample is
    // its own only known value, and saying so is better than inventing a range.
    let range_reported = report["minT"].as_f64().is_some() && report["maxT"].as_f64().is_some();
    let min_c = report["minT"].as_f64().unwrap_or(temp_c);
    let max_c = report["maxT"].as_f64().unwrap_or(temp_c);
    let observed = report["obsTime"]
        .as_i64()
        .ok_or_else(|| Error::Json(format!("{station}: METAR has no obsTime")))?;
    let offset_seconds = utc_offset_min.saturating_mul(60);
    let timezone = if utc_offset_min == 0 {
        "UTC".to_string()
    } else {
        format!(
            "UTC{:+}:{:02}",
            utc_offset_min / 60,
            utc_offset_min.rem_euclid(60)
        )
    };
    let raw = report["rawOb"].as_str().unwrap_or_default();
    let location = label
        .map(str::to_string)
        .or_else(|| report["name"].as_str().map(str::to_string))
        .unwrap_or_else(|| station.to_ascii_uppercase());

    Ok(CurrentWeather {
        range_reported,
        location,
        latitude: report["lat"].as_f64().unwrap_or(0.0),
        longitude: report["lon"].as_f64().unwrap_or(0.0),
        timestamp: observed.saturating_add(i64::from(offset_seconds)),
        utc_offset_seconds: offset_seconds,
        timezone,
        temp_c,
        min_c,
        max_c,
        icon: icon(report, raw),
        sunrise_min: SUN_UNKNOWN,
        sunset_min: SUN_UNKNOWN,
        fetched_at_monotonic_ms,
        forecast: Vec::new(),
    })
}

/// Sky cover from the report's `cover`, sharpened by precipitation in the raw
/// METAR. A descriptor we do not know falls back to the cover code, never to
/// "clear".
fn icon(report: &Value, raw: &str) -> Icon {
    let raw = raw.to_ascii_uppercase();
    let has = |token: &str| raw.contains(token);
    if has("TS") {
        return Icon::Thunderstorm;
    }
    if has("SN") || has("SG") {
        return Icon::Snow;
    }
    if has("RA") || has("DZ") || has("SH") || has("FZRA") {
        return Icon::Rain;
    }
    if has("FG") || has("BR") || has("HZ") {
        return Icon::Mist;
    }
    match report["cover"].as_str().unwrap_or_default() {
        "CLR" | "SKC" => Icon::Clear,
        "FEW" => Icon::FewClouds,
        "SCT" => Icon::Clouds,
        "BKN" => Icon::HeavyClouds,
        "OVC" => Icon::HeavyClouds,
        _ => Icon::Clouds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One live KSEA report, trimmed to the fields this parser reads.
    const KSEA: &str = r#"[{"icaoId":"KSEA","obsTime":1794916380,
        "temp":6.1,"dewp":3.9,"maxT":8.3,"minT":5,"cover":"OVC",
        "rawOb":"METAR KSEA 171153Z 13004KT 10SM BKN028 OVC055 06/04 A3004",
        "lat":47.4447,"lon":-122.3144,"name":"Seattle-Tacoma Intl, WA, US"}]"#;

    #[test]
    fn a_station_report_becomes_an_observation() {
        let weather = parse(KSEA.as_bytes(), "KSEA", None, 42, 0).expect("parse");
        assert_eq!(weather.temp_c, 6.1);
        assert_eq!(weather.min_c, 5.0, "six-hour extreme, not the current temp");
        assert_eq!(weather.max_c, 8.3);
        assert!(weather.range_reported, "the station published both extremes");
        assert_eq!(weather.icon, Icon::HeavyClouds);
        assert_eq!(weather.timestamp, 1_794_916_380, "METAR is UTC");
        assert_eq!(weather.timezone, "UTC");
        assert!(weather.forecast.is_empty(), "a METAR has no forecast");
        assert_eq!(weather.location, "Seattle-Tacoma Intl, WA, US");
    }

    #[test]
    fn a_station_without_extremes_is_an_observation_not_a_range() {
        let body = r#"[{"icaoId":"KSEA","obsTime":1,"temp":6.1,"cover":"OVC",
            "rawOb":"METAR KSEA 171153Z 13004KT 10SM OVC055 06/04 A3004","lat":47.0,"lon":-122.0}]"#;
        let weather = parse(body.as_bytes(), "KSEA", None, 0, 0).expect("parse");
        assert_eq!(weather.min_c, 6.1, "the only value measured, repeated");
        assert_eq!(weather.max_c, 6.1);
        assert!(
            !weather.range_reported,
            "a repeated single value is not a range"
        );
    }

    #[test]
    fn the_viewer_offset_is_carried_not_invented() {
        let weather = parse(KSEA.as_bytes(), "KSEA", Some("Home"), 42, -480).expect("parse");
        assert_eq!(weather.utc_offset_seconds, -28_800);
        assert_eq!(weather.timestamp, 1_794_916_380 - 28_800);
        assert_eq!(weather.timezone, "UTC-8:00");
        assert_eq!(weather.location, "Home");
    }

    #[test]
    fn precipitation_wins_over_the_cover_code() {
        let snow = r#"[{"icaoId":"KSEA","obsTime":1,"temp":-1.0,"cover":"OVC",
            "rawOb":"METAR KSEA 171153Z 13004KT 1SM -SN OVC010 M01/M02 A3004","lat":47.0,"lon":-122.0}]"#;
        assert_eq!(
            parse(snow.as_bytes(), "KSEA", None, 0, 0).unwrap().icon,
            Icon::Snow
        );

        let fog = r#"[{"icaoId":"KSEA","obsTime":1,"temp":4.0,"cover":"CLR",
            "rawOb":"METAR KSEA 171153Z 00000KT 1/4SM FG CLR 04/03 A3004","lat":47.0,"lon":-122.0}]"#;
        assert_eq!(
            parse(fog.as_bytes(), "KSEA", None, 0, 0).unwrap().icon,
            Icon::Mist
        );
    }

    #[test]
    fn a_report_without_a_temperature_is_an_error_not_a_zero() {
        let body = r#"[{"icaoId":"KSEA","obsTime":1,"cover":"CLR","rawOb":"METAR KSEA","lat":47.0,"lon":-122.0}]"#;
        assert!(parse(body.as_bytes(), "KSEA", None, 0, 0).is_err());
    }

    #[test]
    fn an_empty_report_list_is_an_error() {
        assert!(parse(b"[]", "KSEA", None, 0, 0).is_err());
    }
}
