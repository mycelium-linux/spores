//! The wasm guest: bounded HTTP JSON-RPC in, normalized agents out.

wit_bindgen::generate!({
    path: "wit",
    world: "acp-agents",
    generate_all,
});

use serde_json::{Value, json};
use soil::http::http as http;
use soil::vfs::vfs;

use exports::fungos::acp_agents::{control, dispatch, metrics, observe};
use observe::Guest as _;
mod types {
    pub use super::observe::{
        Agent, Binding, Device, Error, Escalation, Page, Phase, Scope, ScopeKind, Usage,
    };
}

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8787/acp";
const CONFIG_PATH: &str = "agents/acp.json";
const MAX_BODY: u32 = 16 * 1024 * 1024;
const TIMEOUT_MS: u32 = 120_000;

struct Component;

export!(Component);

struct Client {
    endpoint: String,
    harness: String,
    models: Vec<String>,
}

impl Client {
    fn open() -> Result<Self, types::Error> {
        let endpoint = endpoint().unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        let hello = rpc(&endpoint, "initialize", json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
        }))
        .map_err(map_error)?;
        let harness = hello
            .get("agentInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("acp")
            .to_string();
        let models = models_from(&hello);
        Ok(Self { endpoint, harness, models })
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value, types::Error> {
        rpc(&self.endpoint, method, params).map_err(map_error)
    }

    fn agent(&self, session: &Value) -> types::Agent {
        let id = session
            .get("sessionId")
            .or_else(|| session.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let title = session
            .get("title")
            .or_else(|| session.get("name"))
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&id)
            .to_string();
        let workspace = session.get("cwd").and_then(Value::as_str).map(str::to_string);
        let model = session
            .get("model")
            .or_else(|| session.get("currentModel"))
            .and_then(Value::as_str)
            .map(str::to_string);
        types::Agent {
            id,
            harness: self.harness.clone(),
            title,
            workspace,
            model,
            device: None,
            avatar: None,
            phase: types::Phase::Idle,
            goal: None,
            owner: None,
            activity: Vec::new(),
            prompts: Vec::new(),
            usage: types::Usage {
                context_tokens: 0,
                context_window: 0,
                input_tokens: 0,
                output_tokens: 0,
                turns: 0,
                tool_calls: 0,
                tool_errors: 0,
                model_time_ms: 0,
            },
            revision: 0,
            started_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn sessions(&self) -> Result<Vec<Value>, types::Error> {
        let result = self.rpc("session/list", json!({}))?;
        Ok(result
            .get("sessions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }
}

impl observe::Guest for Component {
    fn agents() -> Result<Vec<types::Agent>, types::Error> {
        let client = Client::open()?;
        Ok(client
            .sessions()?
            .iter()
            .map(|session| client.agent(session))
            .collect())
    }

    fn escalations() -> Result<Vec<types::Escalation>, types::Error> {
        Ok(Vec::new())
    }

    fn devices() -> Vec<types::Device> {
        Vec::new()
    }

    fn models() -> Vec<String> {
        Client::open().map(|client| client.models).unwrap_or_default()
    }

    fn get(id: String) -> Result<types::Agent, types::Error> {
        let client = Client::open()?;
        client
            .sessions()?
            .iter()
            .find(|session| {
                session
                    .get("sessionId")
                    .or_else(|| session.get("id"))
                    .and_then(Value::as_str)
                    == Some(id.as_str())
            })
            .map(|session| client.agent(session))
            .ok_or(types::Error::NotFound)
    }

    fn intent() -> types::Binding {
        types::Binding { agent: None, view: None }
    }

    fn current_scope() -> types::Scope {
        types::Scope {
            kind: types::ScopeKind::Device,
            id: None,
            label: None,
            owner: None,
        }
    }

    fn transcript(_id: String, _before: Option<u64>, _limit: u32) -> Result<types::Page, types::Error> {
        // ACP's portable surface is a live session, not a transcript archive.
        // The thread store owns history; this adapter does not invent one.
        Ok(types::Page { turns: Vec::new(), before: None })
    }
}

impl control::Guest for Component {
    fn respond(_id: String, _prompt_id: String, _answer: control::Response) -> Result<(), control::Error> {
        Err(control::Error::Unsupported)
    }

    fn send(id: String, text: String) -> Result<(), control::Error> {
        let client = Client::open().map_err(control_error)?;
        prompt(&client, &id, &text).map(|_| ()).map_err(control_error)
    }

    fn interrupt(id: String) -> Result<(), control::Error> {
        let client = Client::open().map_err(control_error)?;
        client
            .rpc("session/cancel", json!({"sessionId": id}))
            .map(|_| ())
            .map_err(control_error)
    }
}

impl metrics::Guest for Component {
    fn stats(since_ms: Option<u64>, until_ms: Option<u64>) -> Result<metrics::Report, metrics::Error> {
        let agents = Component::agents().map_err(metrics_error)?;
        let since = since_ms.unwrap_or(0);
        let until = until_ms.unwrap_or(u64::MAX);
        let selected: Vec<_> = agents
            .iter()
            .filter(|agent| agent.updated_at_ms >= since && agent.updated_at_ms <= until)
            .collect();
        Ok(metrics::Report {
            since_ms: since,
            until_ms: if until == u64::MAX { 0 } else { until },
            agents: selected.len() as u32,
            active: 0,
            needs_you: 0,
            input_tokens: 0,
            output_tokens: 0,
            turns: 0,
            tool_calls: 0,
            tool_errors: 0,
            tokens_per_second_milli: 0,
            measured_agents: 0,
            by_harness: Vec::new(),
        })
    }
}

impl dispatch::Guest for Component {
    fn dispatch(task: dispatch::Task) -> Result<dispatch::Started, dispatch::Error> {
        let client = Client::open().map_err(dispatch_error)?;
        let created = client
            .rpc("session/new", json!({
                "cwd": "/",
                "mcpServers": [],
            }))
            .map_err(dispatch_error)?;
        let id = created
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or(dispatch::Error::Io)?
            .to_string();
        prompt(&client, &id, &task.text).map_err(dispatch_error)?;
        Ok(dispatch::Started {
            agent_id: id.clone(),
            conversation: id,
        })
    }
}

impl exports::fungos::acp_agents::cli::Guest for Component {
    fn run(args: Vec<String>) -> Result<String, String> {
        let mut endpoint = None;
        let mut words = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--endpoint" => endpoint = iter.next().cloned(),
                other => words.push(other.to_string()),
            }
        }
        if let Some(endpoint) = endpoint {
            // The endpoint is host-approved; this only lets the probe name it.
            let client = Client::open().map_err(describe)?;
            let _ = endpoint;
            return match words.first().map(String::as_str) {
                Some("agents") | None => serde_json::to_string(&client.sessions().map_err(describe)?)
                    .map_err(|error| error.to_string()),
                Some("models") => serde_json::to_string(&client.models).map_err(|error| error.to_string()),
                Some(other) => Err(format!("unknown command: {other}")),
            };
        }
        let client = Client::open().map_err(describe)?;
        match words.first().map(String::as_str) {
            Some("agents") | Some("sessions") | None => {
                let agents: Vec<Value> = client
                    .sessions()
                    .map_err(describe)?
                    .iter()
                    .map(|session| {
                        json!({
                            "id": session.get("sessionId").or_else(|| session.get("id")),
                            "harness": client.harness,
                            "title": session.get("title").or_else(|| session.get("name")),
                        })
                    })
                    .collect();
                serde_json::to_string(&agents).map_err(|error| error.to_string())
            }
            Some("models") => serde_json::to_string(&client.models).map_err(|error| error.to_string()),
            Some("prompt") if words.len() >= 3 => {
                prompt(&client, &words[1], &words[2..].join(" ")).map_err(describe)?;
                Ok("ok".into())
            }
            Some(other) => Err(format!("unknown command: {other}")),
        }
    }
}

fn prompt(client: &Client, id: &str, text: &str) -> Result<Value, types::Error> {
    client.rpc("session/prompt", json!({
        "sessionId": id,
        "prompt": [{"type": "text", "text": text}],
    }))
}

fn endpoint() -> Option<String> {
    let result = vfs::read(CONFIG_PATH, 0, 65536).ok()?;
    let text = String::from_utf8(result.data).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("endpoint")
        .and_then(Value::as_str)
        .filter(|endpoint| endpoint.starts_with("http://") || endpoint.starts_with("https://"))
        .map(|endpoint| endpoint.trim_end_matches('/').to_string())
}

fn rpc(endpoint: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    }))
    .map_err(|error| error.to_string())?;
    let response = http::fetch(&http::Request {
        url: endpoint.to_string(),
        method: http::Method::Post,
        headers: vec![
            http::Header { name: "content-type".into(), value: "application/json".into() },
            http::Header { name: "accept".into(), value: "application/json".into() },
        ],
        body: Some(body),
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: TIMEOUT_MS,
    })
    .map_err(|error| format!("http: {error:?}"))?;
    let text = String::from_utf8(response.body).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_str(text.trim()).map_err(|error| format!("json: {error}"))?;
    if let Some(error) = value.get("error") {
        return Err(error.to_string());
    }
    Ok(value.get("result").cloned().unwrap_or(value))
}

fn models_from(hello: &Value) -> Vec<String> {
    hello
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    model
                        .as_str()
                        .or_else(|| model.get("modelId").and_then(Value::as_str))
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn map_error(error: String) -> types::Error {
    if error.contains("denied") {
        types::Error::Denied
    } else if error.contains("not found") || error.contains("not-found") {
        types::Error::NotFound
    } else if error.contains("invalid") {
        types::Error::Invalid
    } else if error.contains("unsupported") {
        types::Error::Unsupported
    } else {
        types::Error::Io
    }
}

fn control_error(error: types::Error) -> control::Error {
    match error {
        types::Error::Denied => control::Error::Denied,
        types::Error::NotFound => control::Error::NotFound,
        types::Error::NotReady => control::Error::NotReady,
        types::Error::Invalid => control::Error::Invalid,
        types::Error::OverBudget => control::Error::OverBudget,
        types::Error::Stale => control::Error::Stale,
        types::Error::Unsupported => control::Error::Unsupported,
        types::Error::Io => control::Error::Io,
    }
}

fn metrics_error(error: types::Error) -> metrics::Error {
    match error {
        types::Error::Denied => metrics::Error::Denied,
        types::Error::NotFound => metrics::Error::NotFound,
        types::Error::NotReady => metrics::Error::NotReady,
        types::Error::Invalid => metrics::Error::Invalid,
        types::Error::OverBudget => metrics::Error::OverBudget,
        types::Error::Stale => metrics::Error::Stale,
        types::Error::Unsupported => metrics::Error::Unsupported,
        types::Error::Io => metrics::Error::Io,
    }
}

fn dispatch_error(error: types::Error) -> dispatch::Error {
    match error {
        types::Error::Denied => dispatch::Error::Denied,
        types::Error::NotFound => dispatch::Error::NotFound,
        types::Error::NotReady => dispatch::Error::NotReady,
        types::Error::Invalid => dispatch::Error::Invalid,
        types::Error::OverBudget => dispatch::Error::OverBudget,
        types::Error::Stale => dispatch::Error::Stale,
        types::Error::Unsupported => dispatch::Error::Unsupported,
        types::Error::Io => dispatch::Error::Io,
    }
}

fn describe(error: types::Error) -> String {
    match error {
        types::Error::Denied => "denied",
        types::Error::NotFound => "not-found",
        types::Error::NotReady => "not-ready",
        types::Error::Invalid => "invalid",
        types::Error::OverBudget => "over-budget",
        types::Error::Stale => "stale",
        types::Error::Unsupported => "unsupported",
        types::Error::Io => "io",
    }
    .to_string()
}
