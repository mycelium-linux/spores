//! The wasm guest: `soil:http` in, `run(args)` and the typed `lights` interface out.
//!
//! This module is compiled only for `wasm32`, so the command logic in `lib.rs`
//! stays testable on the host without a wasm runtime.

wit_bindgen::generate!({ path: "wit", world: "hue", generate_all });

use cli_http::{Http, Method, Request, Response};
use hue_client::{Bridge, brightness as brightness_body, color as color_body, parse_hex, rgb_to_xy};
use lights_core::Snapshot;
use serde_json::json;

use exports::fungos::hue::lights::{
    Bridge as BridgeHandle, Error as LightsError, Guest as LightsGuest,
    GuestBridge as LightsBridge, Target,
};

struct Component;

export!(Component);

impl Guest for Component {
    fn run(args: Vec<String>) -> Result<String, String> {
        crate::run(&args, &mut HostHttp)
    }
}

impl LightsGuest for Component {
    type Bridge = HueBridge;
}

struct HueBridge {
    bridge: Bridge,
}

impl LightsBridge for HueBridge {
    fn connect(base: String, credential: String) -> Result<BridgeHandle, LightsError> {
        let bridge = Bridge::with_credential(base, credential).map_err(LightsError::Invalid)?;
        Ok(BridgeHandle::new(HueBridge { bridge }))
    }

    fn snapshot(&self) -> Result<exports::fungos::hue::lights::Snapshot, LightsError> {
        let value = self.bridge.snapshot(&mut HostHttp).map_err(bridge_error)?;
        let core = Snapshot::parse(&value).map_err(LightsError::Invalid)?;

        let mut rooms = std::collections::BTreeMap::new();
        for group in &core.groups {
            for id in &group.light_ids {
                rooms.entry(id.clone()).or_insert_with(|| group.name.clone());
            }
        }

        let lights = core
            .lights
            .values()
            .map(|light| exports::fungos::hue::lights::Light {
                id: light.id.clone(),
                name: light.name.clone(),
                on: light.on,
                brightness: light.brightness,
                reachable: light.reachable,
                room: rooms.get(&light.id).cloned(),
            })
            .collect();

        let groups = core
            .groups
            .iter()
            .map(|group| exports::fungos::hue::lights::Group {
                id: group.id.clone(),
                name: group.name.clone(),
                kind: group.kind.clone(),
                on: group.on,
                brightness: group.brightness,
                lights: group.light_ids.clone(),
            })
            .collect();

        let scenes = core
            .scenes
            .iter()
            .map(|scene| exports::fungos::hue::lights::Scene {
                id: scene.id.clone(),
                name: scene.name.clone(),
                group: scene.group.clone(),
            })
            .collect();

        Ok(exports::fungos::hue::lights::Snapshot {
            bridge: core.bridge_name,
            lights,
            groups,
            scenes,
        })
    }

    fn set_power(&self, target: Target, on: bool) -> Result<(), LightsError> {
        self.bridge
            .put(&mut HostHttp, &action_path(&target), json!({ "on": on }))
            .map(|_| ())
            .map_err(bridge_error)
    }

    fn set_brightness(&self, target: Target, value: u8) -> Result<(), LightsError> {
        let value = value.clamp(1, 254);
        self.bridge
            .put(&mut HostHttp, &action_path(&target), brightness_body(value))
            .map(|_| ())
            .map_err(bridge_error)
    }

    fn set_colour(&self, target: Target, hex: String) -> Result<(), LightsError> {
        let (r, g, b) = parse_hex(&hex).map_err(LightsError::Invalid)?;
        let _ = rgb_to_xy(r, g, b);
        self.bridge
            .put(&mut HostHttp, &action_path(&target), color_body(r, g, b))
            .map(|_| ())
            .map_err(bridge_error)
    }

    fn set_scene(&self, group: String, scene: String) -> Result<(), LightsError> {
        self.bridge
            .put(
                &mut HostHttp,
                &format!("/groups/{group}/action"),
                json!({ "scene": scene }),
            )
            .map(|_| ())
            .map_err(bridge_error)
    }
}

fn action_path(target: &Target) -> String {
    match target {
        Target::Group(id) => format!("/groups/{id}/action"),
        Target::Light(id) => format!("/lights/{id}/state"),
    }
}

fn bridge_error(message: String) -> LightsError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("denied") || lower.contains("401") || lower.contains("403") {
        LightsError::Denied(message)
    } else if lower.contains("unavailable") || lower.contains("timeout") || lower.contains("http 5") {
        LightsError::Unavailable(message)
    } else {
        LightsError::Bridge(message)
    }
}

struct HostHttp;

impl Http for HostHttp {
    fn fetch(&mut self, request: Request) -> Result<Response, String> {
        use soil::http::http as abi;

        let method = match request.method {
            Method::Get => abi::Method::Get,
            Method::Put => abi::Method::Put,
            Method::Post => abi::Method::Post,
        };
        let response = abi::fetch(&abi::Request {
            url: request.url,
            method,
            headers: request
                .headers
                .into_iter()
                .map(|(name, value)| abi::Header { name, value })
                .collect(),
            body: request.body,
            credential: request.credential,
            max_bytes: crate::MAX_BODY,
            timeout_ms: crate::TIMEOUT_MS,
        })
        .map_err(|error| format!("http: {error:?}"))?;

        Ok(Response {
            status: response.status,
            body: response.body,
        })
    }
}
