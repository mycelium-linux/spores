//! COLMI ring (QRing) wire formats.
//!
//! The ring is not a watch: it has no screen, no music service, and no clock
//! UI. Everything it says arrives on one UART service as fixed 16-byte
//! packets, and most of it is request/response rather than a stream. This
//! crate owns that wire; the Spore owns the radio session and the record.
//!
//! Sources (reverse-engineered, checked against captures):
//! - <https://github.com/tahnok/colmi_r02_client/blob/main/colmi_r02_client/packet.py>
//! - <https://github.com/tahnok/colmi_r02_client/blob/main/colmi_r02_client/client.py>
//! - <https://github.com/tahnok/colmi_r02_client/blob/main/colmi_r02_client/real_time.py>
//! - <https://github.com/tahnok/colmi_r02_client/blob/main/colmi_r02_client/battery.py>
//! - <https://github.com/tahnok/colmi_r02_client/blob/main/colmi_r02_client/steps.py>
//! - <https://colmi.puxtril.com/commands/>
//!
//! Nothing here imports wasm, so every byte layout is testable on the host.

use std::fmt;

/// Nordic-style UART service the ring speaks. The ring is the peripheral: the
/// central writes `UART_WRITE` and receives `UART_NOTIFY`.
pub const UART_SERVICE: &str = "6e40fff0-b5a3-f393-e0a9-e50e24dcca9e";
pub const UART_WRITE: &str = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";
pub const UART_NOTIFY: &str = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";

/// Firmware-update service exposed by the RF03 bootloader in this ring family.
/// It is a separate channel from the UART command set above, and this crate
/// only names it: a probe reports what the service exposes, and any future
/// writer lives in its own module.
///
/// Source: <https://github.com/atc1441/ATC_RF03_Ring> (bootloader dump,
/// firmware images, and the browser flasher that drives these UUIDs).
pub const OTA_SERVICE: &str = "de5bf728-d711-4e47-af26-65e3012a5dc7";
pub const OTA_WRITE: &str = "de5bf72a-d711-4e47-af26-65e3012a5dc7";
pub const OTA_NOTIFY: &str = "de5bf729-d711-4e47-af26-65e3012a5dc7";

/// Vendor service that sits beside the UART and OTA services on these rings.
/// Purpose unknown; named so a probe shows it and nobody mistakes it for OTA.
pub const VENDOR_SERVICE: &str = "0000fee7-0000-1000-8000-00805f9b34fb";

/// Standard Device Information service. A ring that exposes it answers here
/// instead of inventing its own identity characteristic.
pub const DEVICE_INFO_SERVICE: &str = "0000180a-0000-1000-8000-00805f9b34fb";
pub const HARDWARE_REVISION: &str = "00002a27-0000-1000-8000-00805f9b34fb";
pub const FIRMWARE_REVISION: &str = "00002a26-0000-1000-8000-00805f9b34fb";
pub const SERIAL_NUMBER: &str = "00002a25-0000-1000-8000-00805f9b34fb";
pub const MODEL_NUMBER: &str = "00002a24-0000-1000-8000-00805f9b34fb";

/// Every packet is exactly this long, request and response alike.
pub const PACKET_LEN: usize = 16;

pub const CMD_BATTERY: u8 = 3;
pub const CMD_GET_STEPS: u8 = 67;
pub const CMD_START_REAL_TIME: u8 = 105;
pub const CMD_STOP_REAL_TIME: u8 = 106;

/// A response with this bit set reports a device error, not a measurement.
pub const ERROR_BIT: u8 = 0x80;

/// The `start` action byte. `pause`, `continue` and `stop` exist in the
/// protocol but a session that only streams does not need them.
const ACTION_START: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// A request carried more payload than fits in one packet.
    SubDataTooLong(usize),
    /// A response was not exactly [`PACKET_LEN`] bytes.
    WrongLength(usize),
    /// The trailing checksum did not match the packet contents.
    BadChecksum { expected: u8, actual: u8 },
    /// The ring set the error bit and named a code.
    DeviceError { command: u8, code: u8 },
    /// A packet named a reading this profile does not know.
    UnknownReading(u8),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SubDataTooLong(len) => {
                write!(f, "sub-data is {len} bytes, at most 14 fit in a packet")
            }
            Self::WrongLength(len) => write!(f, "packet is {len} bytes, expected {PACKET_LEN}"),
            Self::BadChecksum { expected, actual } => {
                write!(f, "checksum {actual:#04x} does not match {expected:#04x}")
            }
            Self::DeviceError { command, code } => {
                write!(f, "device error {code} for command {command}")
            }
            Self::UnknownReading(kind) => write!(f, "unknown real-time reading {kind}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// The packet checksum: the sum of all bytes, truncated to one byte. It is an
/// additive checksum, not a CRC.
#[must_use]
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}

/// Build a 16-byte request: command, sub-data, zero padding, checksum.
pub fn packet(command: u8, sub: &[u8]) -> Result<[u8; PACKET_LEN], ProtocolError> {
    if sub.len() > PACKET_LEN - 2 {
        return Err(ProtocolError::SubDataTooLong(sub.len()));
    }
    let mut out = [0u8; PACKET_LEN];
    out[0] = command;
    out[1..1 + sub.len()].copy_from_slice(sub);
    // The checksum covers the command and sub-data; padding is zero, so it is
    // the same sum either way.
    out[PACKET_LEN - 1] = checksum(&out);
    Ok(out)
}

/// Validate a response: length, checksum, and the error bit. Returns the
/// command byte so the caller can dispatch without re-checking.
pub fn checked(packet: &[u8]) -> Result<u8, ProtocolError> {
    if packet.len() != PACKET_LEN {
        return Err(ProtocolError::WrongLength(packet.len()));
    }
    let expected = checksum(&packet[..PACKET_LEN - 1]);
    let actual = packet[PACKET_LEN - 1];
    if expected != actual {
        return Err(ProtocolError::BadChecksum { expected, actual });
    }
    let command = packet[0];
    if command & ERROR_BIT != 0 {
        return Err(ProtocolError::DeviceError {
            command: command & !ERROR_BIT,
            code: packet[2],
        });
    }
    Ok(command)
}

/// A measurement the ring can stream in real time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealTime {
    HeartRate,
    BloodPressure,
    Spo2,
    Fatigue,
    HealthCheck,
    Ecg,
    Pressure,
    BloodSugar,
    Hrv,
}

impl RealTime {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::HeartRate => 1,
            Self::BloodPressure => 2,
            Self::Spo2 => 3,
            Self::Fatigue => 4,
            Self::HealthCheck => 5,
            Self::Ecg => 7,
            Self::Pressure => 8,
            Self::BloodSugar => 9,
            Self::Hrv => 10,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, ProtocolError> {
        Ok(match code {
            1 => Self::HeartRate,
            2 => Self::BloodPressure,
            3 => Self::Spo2,
            4 => Self::Fatigue,
            5 => Self::HealthCheck,
            7 => Self::Ecg,
            8 => Self::Pressure,
            9 => Self::BloodSugar,
            10 => Self::Hrv,
            other => return Err(ProtocolError::UnknownReading(other)),
        })
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HeartRate => "heart-rate",
            Self::BloodPressure => "blood-pressure",
            Self::Spo2 => "spo2",
            Self::Fatigue => "fatigue",
            Self::HealthCheck => "health-check",
            Self::Ecg => "ecg",
            Self::Pressure => "pressure",
            Self::BloodSugar => "blood-sugar",
            Self::Hrv => "hrv",
        }
    }
}

/// Ask the ring to start streaming one reading.
pub fn start_real_time(kind: RealTime) -> Result<[u8; PACKET_LEN], ProtocolError> {
    packet(CMD_START_REAL_TIME, &[kind.code(), ACTION_START])
}

/// Stop streaming one reading.
pub fn stop_real_time(kind: RealTime) -> Result<[u8; PACKET_LEN], ProtocolError> {
    packet(CMD_STOP_REAL_TIME, &[kind.code(), 0, 0])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub kind: RealTime,
    /// Raw device units. Heart rate is bpm; SpO2 is percent.
    pub value: u8,
}

/// Decode one real-time packet (`CMD_START_REAL_TIME`).
pub fn parse_real_time(packet: &[u8]) -> Result<Reading, ProtocolError> {
    if checked(packet)? != CMD_START_REAL_TIME {
        return Err(ProtocolError::DeviceError {
            command: packet[0],
            code: 0,
        });
    }
    let kind = RealTime::from_code(packet[1])?;
    let code = packet[2];
    if code != 0 {
        return Err(ProtocolError::DeviceError {
            command: CMD_START_REAL_TIME,
            code,
        });
    }
    Ok(Reading {
        kind,
        value: packet[3],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// Ask for the battery level once.
pub fn battery_request() -> Result<[u8; PACKET_LEN], ProtocolError> {
    packet(CMD_BATTERY, &[])
}

/// Decode a battery response (`CMD_BATTERY`).
pub fn parse_battery(packet: &[u8]) -> Result<Battery, ProtocolError> {
    checked(packet)?;
    Ok(Battery {
        percent: packet[1].min(100),
        charging: packet[2] != 0,
    })
}

/// Ask for one day of step history, `day_offset` days before the ring's today.
pub fn steps_request(day_offset: u8) -> Result<[u8; PACKET_LEN], ProtocolError> {
    // `0f 00 5f 01` are constant bytes the captures show; only the offset
    // varies. They are recorded, not understood.
    packet(CMD_GET_STEPS, &[day_offset, 0x0f, 0x00, 0x5f, 0x01])
}

/// One 15-minute bucket of the ring's activity log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SportDetail {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// Quarter-hour index within the day: `time_index / 4` hours and
    /// `time_index % 4 * 15` minutes, in the ring's own clock.
    pub time_index: u8,
    pub calories: u16,
    pub steps: u16,
    /// Metres.
    pub distance: u16,
}

/// The outcome of feeding one steps packet to [`StepsParser`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepsUpdate {
    /// A packet was consumed and more are coming.
    More,
    /// The ring has no data for the requested day.
    NoData,
    /// The last packet arrived; the day is complete.
    Done(Vec<SportDetail>),
}

/// The steps response is several packets, so the parser holds the page index
/// and what it has read so far.
#[derive(Debug, Default)]
pub struct StepsParser {
    new_calorie_protocol: bool,
    index: usize,
    details: Vec<SportDetail>,
}

impl StepsParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one packet. `Done` and `NoData` reset the parser for the next
    /// request; a packet after either starts a fresh page.
    pub fn push(&mut self, packet: &[u8]) -> Result<StepsUpdate, ProtocolError> {
        checked(packet)?;
        if packet[0] != CMD_GET_STEPS {
            return Err(ProtocolError::DeviceError {
                command: packet[0],
                code: 0,
            });
        }
        if self.index == 0 && packet[1] == 0xff {
            self.reset();
            return Ok(StepsUpdate::NoData);
        }
        if self.index == 0 && packet[1] == 0xf0 {
            self.new_calorie_protocol = packet[3] == 1;
            self.index += 1;
            return Ok(StepsUpdate::More);
        }

        let calories = u16::from_le_bytes([packet[7], packet[8]]);
        self.details.push(SportDetail {
            year: 2000 + u16::from(bcd(packet[1])),
            month: bcd(packet[2]),
            day: bcd(packet[3]),
            time_index: packet[4],
            calories: if self.new_calorie_protocol {
                calories.saturating_mul(10)
            } else {
                calories
            },
            steps: u16::from_le_bytes([packet[9], packet[10]]),
            distance: u16::from_le_bytes([packet[11], packet[12]]),
        });

        // Bytes 5 and 6 are the packet index and the packet count.
        if packet[5].saturating_add(1) == packet[6] {
            let mut details = std::mem::take(&mut self.details);
            self.reset();
            details.sort_by_key(|detail| detail.time_index);
            return Ok(StepsUpdate::Done(details));
        }
        self.index += 1;
        Ok(StepsUpdate::More)
    }

    fn reset(&mut self) {
        self.new_calorie_protocol = false;
        self.index = 0;
        self.details.clear();
    }
}

/// Two decimal digits packed into one byte.
fn bcd(byte: u8) -> u8 {
    ((byte >> 4) & 0x0f) * 10 + (byte & 0x0f)
}

/// The profile the Spore registers with `watch-core`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Profile;

impl watch_core::WatchProfile for Profile {
    fn name(&self) -> &'static str {
        "colmi"
    }

    fn streams(&self) -> &'static [watch_core::Stream] {
        &[
            watch_core::Stream::HeartRate,
            watch_core::Stream::Spo2,
            watch_core::Stream::Steps,
            watch_core::Stream::Battery,
        ]
    }

    fn decode(&self, char_uuid: &str, data: &[u8]) -> Result<Option<watch_core::Signal>, String> {
        if !char_uuid.eq_ignore_ascii_case(UART_NOTIFY) {
            return Ok(None);
        }
        let command = checked(data).map_err(|error| error.to_string())?;
        match command {
            CMD_START_REAL_TIME => {
                let reading = parse_real_time(data).map_err(|error| error.to_string())?;
                let signal = match reading.kind {
                    RealTime::HeartRate => watch_core::Signal {
                        stream: watch_core::Stream::HeartRate,
                        value: serde_json::json!({"bpm": reading.value}),
                        privacy_class: "health",
                        retention_class: "raw",
                        // The ring sends 0 while it has no contact; that is
                        // unknown, not a stopped heart.
                        quality: if reading.value == 0 { 0.0 } else { 1.0 },
                    },
                    RealTime::Spo2 => watch_core::Signal {
                        stream: watch_core::Stream::Spo2,
                        value: serde_json::json!({"percent": reading.value}),
                        privacy_class: "health",
                        retention_class: "raw",
                        quality: if reading.value == 0 { 0.0 } else { 1.0 },
                    },
                    // Other real-time readings exist in the protocol; this
                    // profile does not claim to understand their units.
                    _ => return Ok(None),
                };
                Ok(Some(signal))
            }
            // Battery and steps are request/response: the Spore parses them
            // directly, because they need the request it sent to make sense.
            _ => Ok(None),
        }
    }

    fn supports(&self, _firmware: &str, intent: watch_core::Intent) -> Option<bool> {
        // A ring has no screen to notify, no media control, and no navigation.
        Some(matches!(intent, watch_core::Intent::Watch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden packets from the reference client's test suite.
    #[test]
    fn start_and_stop_match_the_reference_bytes() {
        let start = start_real_time(RealTime::HeartRate).expect("start");
        assert_eq!(start[0], CMD_START_REAL_TIME);
        assert_eq!(start[1], 1);
        assert_eq!(start[2], ACTION_START);
        // 105 + 1 (reading) + 1 (start) = 107.
        assert_eq!(start[15], 0x6b, "checksum is the byte sum");
        assert_eq!(checksum(&start[..15]), start[15]);

        let stop = stop_real_time(RealTime::HeartRate).expect("stop");
        assert_eq!(stop[0], CMD_STOP_REAL_TIME);
        assert_eq!(stop[1], 1);
        assert_eq!(stop[15], 0x6b);
    }

    #[test]
    fn a_real_time_reading_parses() {
        // b"i\x01\x00N" + padding + 0xb8: command 105, HR, no error, 78 bpm.
        let packet = [
            0x69, 0x01, 0x00, 0x4e, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xb8,
        ];
        assert_eq!(
            parse_real_time(&packet).expect("reading"),
            Reading {
                kind: RealTime::HeartRate,
                value: 78
            }
        );
    }

    #[test]
    fn a_device_error_is_an_error_not_a_zero() {
        let packet = [
            0x69, 0x01, 0x01, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x6b,
        ];
        assert_eq!(
            parse_real_time(&packet).expect_err("device error"),
            ProtocolError::DeviceError {
                command: CMD_START_REAL_TIME,
                code: 1
            }
        );
    }

    #[test]
    fn a_corrupt_packet_is_refused() {
        let mut packet = start_real_time(RealTime::Spo2).expect("start");
        packet[3] ^= 0xff;
        assert!(matches!(
            checked(&packet),
            Err(ProtocolError::BadChecksum { .. })
        ));
        assert!(matches!(
            checked(&packet[..12]),
            Err(ProtocolError::WrongLength(12))
        ));
    }

    #[test]
    fn the_error_bit_is_reported_as_a_device_error() {
        let packet = [
            0x80 | CMD_BATTERY,
            0,
            7,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        let packet = {
            let mut copy = packet;
            copy[15] = checksum(&copy[..15]);
            copy
        };
        assert_eq!(
            checked(&packet).expect_err("error bit"),
            ProtocolError::DeviceError {
                command: CMD_BATTERY,
                code: 7
            }
        );
    }

    #[test]
    fn battery_parses_percent_and_charging() {
        // b"\x03@\x00..." — 0x40 = 64%, not charging.
        let mut packet = [0u8; 16];
        packet[0] = CMD_BATTERY;
        packet[1] = 0x40;
        packet[15] = checksum(&packet[..15]);
        assert_eq!(
            parse_battery(&packet).expect("battery"),
            Battery {
                percent: 64,
                charging: false
            }
        );
    }

    #[test]
    fn steps_requests_carry_the_reference_padding() {
        let request = steps_request(0).expect("steps");
        assert_eq!(&request[..6], &[0x43, 0x00, 0x0f, 0x00, 0x5f, 0x01]);
        assert_eq!(request[15], 0xb2);
    }

    #[test]
    fn a_steps_page_parses_until_its_last_packet() {
        // b"C#\x08\x13\x10\x00\x05\xc8\x000\x00\x1b\x00\x00\x00\xa9":
        // 2023-08-19, bucket 16 (04:00), 200 kcal, 48 steps, 27 m, 1 of 1.
        let packet = [
            0x43, 0x23, 0x08, 0x13, 0x10, 0x00, 0x01, 0xc8, 0x00, 0x30, 0x00, 0x1b, 0x00, 0x00,
            0x00, 0x00,
        ];
        let packet = {
            let mut copy = packet;
            copy[15] = checksum(&copy[..15]);
            copy
        };
        let mut parser = StepsParser::new();
        assert_eq!(
            parser.push(&packet).expect("detail"),
            StepsUpdate::Done(vec![SportDetail {
                year: 2023,
                month: 8,
                day: 13,
                time_index: 16,
                calories: 200,
                steps: 48,
                distance: 27,
            }])
        );
    }

    #[test]
    fn a_day_without_data_is_not_a_zero_step_count() {
        let mut packet = [0u8; 16];
        packet[0] = CMD_GET_STEPS;
        packet[1] = 0xff;
        packet[15] = checksum(&packet[..15]);
        let mut parser = StepsParser::new();
        assert_eq!(parser.push(&packet).expect("no data"), StepsUpdate::NoData);
    }

    #[test]
    fn the_profile_decodes_hr_and_spo2_and_leaves_the_rest() {
        use watch_core::WatchProfile;
        let profile = Profile;

        let hr = [0x69, 0x01, 0x00, 61, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let hr = {
            let mut copy = hr;
            copy[15] = checksum(&copy[..15]);
            copy
        };
        let signal = profile
            .decode(UART_NOTIFY, &hr)
            .expect("decode")
            .expect("signal");
        assert_eq!(signal.stream, watch_core::Stream::HeartRate);
        assert_eq!(signal.value["bpm"], 61);

        let spo2 = [0x69, 0x03, 0x00, 97, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let spo2 = {
            let mut copy = spo2;
            copy[15] = checksum(&copy[..15]);
            copy
        };
        assert_eq!(
            profile
                .decode(UART_NOTIFY, &spo2)
                .expect("decode")
                .expect("signal")
                .value["percent"],
            97
        );

        // A characteristic this profile does not own is not an error.
        assert_eq!(profile.decode(UART_WRITE, &hr).expect("other"), None);
        // A ring has no screen, so no screen intent is supported.
        assert_eq!(
            profile.supports("1.0", watch_core::Intent::Notify),
            Some(false)
        );
        assert_eq!(
            profile.supports("1.0", watch_core::Intent::Watch),
            Some(true)
        );
    }
}
