//! The wasm guest: bounded HTTP JSON-RPC in, normalized agents out.

wit_bindgen::generate!({
    path: "wit",
    world: "acp-agents",
    generate_all,
});

use serde_json::{Value, json};
use soil::http::http;
use soil::vfs::vfs;
use soil::vfs_mut::vfs_mut;

use exports::mycelium::acp_agents::{control, dispatch, metrics, observe};
use observe::Guest as _;
mod types {
    pub use super::observe::{
        Agent, Binding, Device, Error, Escalation, Goal, Page, Phase, Role, Scope, ScopeKind, Turn,
        Usage,
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
    sse: Option<String>,
    workspace: String,
    harness: String,
    models: Vec<String>,
}

#[derive(Default)]
struct Config {
    endpoint: Option<String>,
    sse: Option<String>,
    workspace: Option<String>,
}

impl Client {
    fn open() -> Result<Self, types::Error> {
        let config = config();
        let endpoint = config
            .endpoint
            .clone()
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        let hello = rpc(
            &endpoint,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {},
            }),
        )
        .map_err(map_error)?;
        let harness = hello
            .get("agentInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("acp")
            .to_string();
        let models = models_from(&hello);
        Ok(Self {
            endpoint,
            sse: config.sse,
            workspace: config.workspace.unwrap_or_else(|| "/".to_string()),
            harness,
            models,
        })
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value, types::Error> {
        rpc(&self.endpoint, method, params).map_err(map_error)
    }

    fn sse_url(&self, session: &str) -> String {
        let base = self.sse.clone().unwrap_or_else(|| {
            if let Some(prefix) = self.endpoint.strip_suffix("/acp") {
                format!("{prefix}/sse")
            } else {
                format!("{}/sse", self.endpoint.trim_end_matches('/'))
            }
        });
        if base.contains('?') {
            format!("{base}&sessionId={session}")
        } else {
            format!("{base}?sessionId={session}")
        }
    }

    fn agent(&self, session: &Value) -> types::Agent {
        let id = session
            .get("sessionId")
            .or_else(|| session.get("session_id"))
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
        let workspace = session
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string);
        let model = session
            .get("model")
            .or_else(|| session.get("currentModel"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let usage = session.get("usage");
        let usage_num = |name: &str| usage.and_then(|value| value.get(name)).and_then(Value::as_u64).unwrap_or_default();
        let goal = session.get("goal").and_then(|value| {
            let text = value.get("text").and_then(Value::as_str)?;
            Some(types::Goal {
                text: text.to_string(),
                set_at_ms: value.get("setAtMs").or_else(|| value.get("set_at_ms")).and_then(Value::as_u64).unwrap_or_default(),
            })
        });
        types::Agent {
            id,
            harness: self.harness.clone(),
            title,
            workspace,
            model,
            device: session.get("device").and_then(Value::as_str).map(str::to_string),
            avatar: None,
            phase: agent_phase(session.get("phase").and_then(Value::as_str)),
            goal,
            owner: None,
            activity: Vec::new(),
            prompts: Vec::new(),
            usage: types::Usage {
                context_tokens: usage_num("contextTokens"),
                context_window: usage_num("contextWindow"),
                input_tokens: usage_num("inputTokens"),
                output_tokens: usage_num("outputTokens"),
                turns: usage_num("turns") as u32,
                tool_calls: usage_num("toolCalls"),
                tool_errors: usage_num("toolErrors"),
                model_time_ms: usage_num("modelTimeMs"),
            },
            revision: session.get("revision").and_then(Value::as_u64).unwrap_or_default(),
            started_at_ms: session.get("startedAtMs").or_else(|| session.get("started_at_ms")).and_then(Value::as_u64).unwrap_or_default(),
            updated_at_ms: session.get("updatedAtMs").or_else(|| session.get("updated_at")).and_then(Value::as_u64).unwrap_or_default(),
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
        Client::open()
            .map(|client| client.models)
            .unwrap_or_default()
    }

    fn get(id: String) -> Result<types::Agent, types::Error> {
        let client = Client::open()?;
        client
            .sessions()?
            .iter()
            .find(|session| {
                session
                    .get("sessionId")
                    .or_else(|| session.get("session_id"))
                    .or_else(|| session.get("id"))
                    .and_then(Value::as_str)
                    == Some(id.as_str())
            })
            .map(|session| client.agent(session))
            .ok_or(types::Error::NotFound)
    }

    fn intent() -> types::Binding {
        types::Binding {
            agent: None,
            view: None,
        }
    }

    fn current_scope() -> types::Scope {
        types::Scope {
            kind: types::ScopeKind::Device,
            id: None,
            label: None,
            owner: None,
        }
    }

    fn transcript(
        id: String,
        before: Option<u64>,
        limit: u32,
    ) -> Result<types::Page, types::Error> {
        let client = Client::open()?;
        let mut params = json!({"sessionId": id, "limit": limit.clamp(1, 500)});
        if let Some(before) = before {
            params["before"] = json!(before);
        }
        let result = client.rpc("session/transcript", params)?;
        let turns = result
            .get("turns")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|row| types::Turn {
                        role: match row.get("role").and_then(Value::as_str) {
                            Some("user") => types::Role::User,
                            Some("tool") => types::Role::Tool,
                            _ => types::Role::Assistant,
                        },
                        text: row
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        at_ms: row.get("atMs").and_then(Value::as_u64).unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(types::Page {
            turns,
            before: result.get("before").and_then(Value::as_u64),
        })
    }
}

impl control::Guest for Component {
    fn respond(
        _id: String,
        _prompt_id: String,
        _answer: control::Response,
    ) -> Result<(), control::Error> {
        Err(control::Error::Unsupported)
    }

    fn send(id: String, text: String) -> Result<(), control::Error> {
        let client = Client::open().map_err(control_error)?;
        prompt(&client, &id, &text)
            .map(|_| ())
            .map_err(control_error)
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
    fn stats(
        since_ms: Option<u64>,
        until_ms: Option<u64>,
    ) -> Result<metrics::Report, metrics::Error> {
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
            .rpc(
                "session/new",
                json!({
                    "cwd": if client.workspace == "/" { "/Users/ajmwagar/src/singularity" } else { &client.workspace },
                    "name": "theseus-dispatch",
                    "mcpServers": [],
                }),
            )
            .map_err(dispatch_error)?;
        let id = created
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or(dispatch::Error::Io)?
            .to_string();
        if let Some(model) = task.model.as_deref() {
            client
                .rpc(
                    "session/set_mode",
                    json!({"sessionId": id, "modeId": model}),
                )
                .map_err(dispatch_error)?;
        }
        let reply = stream_prompt(&client, &id, &task.text).map_err(dispatch_error)?;
        let _ = reply;
        Ok(dispatch::Started {
            agent_id: id.clone(),
            conversation: id,
        })
    }
}

impl exports::mycelium::acp_agents::cli::Guest for Component {
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
                Some("agents") | None => {
                    serde_json::to_string(&client.sessions().map_err(describe)?)
                        .map_err(|error| error.to_string())
                }
                Some("models") => {
                    serde_json::to_string(&client.models).map_err(|error| error.to_string())
                }
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
                            "id": session
                                .get("sessionId")
                                .or_else(|| session.get("session_id"))
                                .or_else(|| session.get("id")),
                            "harness": client.harness,
                            "title": session.get("title").or_else(|| session.get("name")),
                            "workspace": session.get("cwd"),
                            "model": session.get("model").or_else(|| session.get("currentModel")),
                        })
                    })
                    .collect();
                serde_json::to_string(&agents).map_err(|error| error.to_string())
            }
            Some("models") => {
                serde_json::to_string(&client.models).map_err(|error| error.to_string())
            }
            Some("dispatch") if words.len() >= 2 => {
                let mut at = 1;
                let mut model = None;
                if words[at] == "--model" && words.len() > at + 1 {
                    model = Some(words[at + 1].clone());
                    at += 2;
                }
                let text = words[at..].join(" ");
                let created = client
                    .rpc(
                        "session/new",
                        json!({"cwd": client.workspace, "mcpServers": []}),
                    )
                    .map_err(describe)?;
                let id = created
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "session/new returned no id".to_string())?
                    .to_string();
                if let Some(model) = model {
                    client
                        .rpc(
                            "session/set_mode",
                            json!({"sessionId": id, "modeId": model}),
                        )
                        .map_err(describe)?;
                }
                let reply = stream_prompt(&client, &id, &text).map_err(describe)?;
                serde_json::to_string(&json!({"agentId": id, "reply": reply}))
                    .map_err(|error| error.to_string())
            }
            Some("create") if words.len() >= 4 => {
                let cwd = words[1].clone();
                let name = words[2].clone();
                let mut at = 3;
                let mut model = None;
                if words[at] == "--model" && words.len() > at + 1 {
                    model = Some(words[at + 1].clone());
                    at += 2;
                }
                let task = words[at..].join(" ");
                let mut create = json!({"cwd": cwd, "name": name, "provider": "pi"});
                if let Some(model) = model {
                    create["model"] = json!(model);
                }
                let created = client
                    .rpc("session/new", create)
                    .map_err(describe)?;
                let id = created
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "session/new returned no id".to_string())?
                    .to_string();
                prompt(&client, &id, &task).map_err(describe)?;
                serde_json::to_string(&json!({
                    "status": "accepted",
                    "agentId": id,
                    "sessionId": id,
                    "created": created,
                }))
                .map_err(|error| error.to_string())
            }
            Some("status") if words.len() >= 2 => {
                let id = words[1].clone();
                let value = client
                    .rpc("session/status", json!({"sessionId": id}))
                    .map_err(describe)?;
                let phase = agent_phase(value.get("phase").and_then(Value::as_str));
                let usage = value.get("usage");
                let usage_num = |name: &str| usage.and_then(|value| value.get(name)).and_then(Value::as_u64).unwrap_or_default();
                serde_json::to_string(&json!({
                    "id": value.get("id").or_else(|| value.get("agentId")).and_then(Value::as_str).unwrap_or(&id),
                    "harness": value.get("harness"),
                    "title": value.get("title"),
                    "workspace": value.get("workspace"),
                    "model": value.get("model"),
                    "phase": phase_name(phase),
                    "goal": value.get("goal"),
                    "usage": {
                        "contextTokens": usage_num("contextTokens"),
                        "inputTokens": usage_num("inputTokens"),
                        "outputTokens": usage_num("outputTokens"),
                        "turns": usage_num("turns"),
                        "toolCalls": usage_num("toolCalls"),
                        "toolErrors": usage_num("toolErrors"),
                        "modelTimeMs": usage_num("modelTimeMs"),
                    },
                    "revision": value.get("revision"),
                    "startedAtMs": value.get("startedAtMs"),
                    "updatedAtMs": value.get("updatedAtMs"),
                    "lastTurn": null,
                    "terminal": matches!(phase, types::Phase::Finished | types::Phase::Failed | types::Phase::Paused),
                }))
                .map_err(|error| error.to_string())
            }
            Some("prompt") if words.len() >= 3 => {
                prompt(&client, &words[1], &words[2..].join(" ")).map_err(describe)?;
                Ok("ok".into())
            }
            Some("interrupt") if words.len() >= 2 => {
                client
                    .rpc("session/cancel", json!({"sessionId": words[1]}))
                    .map_err(describe)?;
                Ok(json!({"status": "queued"}).to_string())
            }
            Some("transcript") if words.len() >= 2 => {
                let page = <Component as observe::Guest>::transcript(
                    words[1].clone(),
                    None,
                    words.get(2).and_then(|value| value.parse().ok()).unwrap_or(50),
                )
                .map_err(describe)?;
                let turns: Vec<Value> = page
                    .turns
                    .iter()
                    .map(|turn| {
                        json!({
                            "role": match turn.role {
                                types::Role::User => "user",
                                types::Role::Tool => "tool",
                                types::Role::Assistant => "assistant",
                            },
                            "text": turn.text,
                            "atMs": turn.at_ms,
                        })
                    })
                    .collect();
                serde_json::to_string(&json!({"turns": turns, "before": page.before}))
                    .map_err(|error| error.to_string())
            }
            Some(other) => Err(format!("unknown command: {other}")),
        }
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > max {
            break;
        }
        out.push(ch);
    }
    out
}

fn agent_phase(value: Option<&str>) -> types::Phase {
    match value.unwrap_or("idle") {
        "connecting" => types::Phase::Connecting,
        "thinking" => types::Phase::Thinking,
        "working" => types::Phase::Working,
        "needs-you" => types::Phase::NeedsYou,
        "finished" => types::Phase::Finished,
        "failed" => types::Phase::Failed,
        "paused" => types::Phase::Paused,
        _ => types::Phase::Idle,
    }
}

fn phase_name(value: types::Phase) -> &'static str {
    match value {
        types::Phase::Connecting => "connecting",
        types::Phase::Idle => "idle",
        types::Phase::Thinking => "thinking",
        types::Phase::Working => "working",
        types::Phase::NeedsYou => "needs-you",
        types::Phase::Finished => "finished",
        types::Phase::Failed => "failed",
        types::Phase::Paused => "paused",
    }
}

fn prompt(client: &Client, id: &str, text: &str) -> Result<Value, types::Error> {
    client.rpc(
        "session/prompt",
        json!({
            "sessionId": id,
            "prompt": [{"type": "text", "text": text}],
        }),
    )
}

fn config() -> Config {
    let mut config = Config::default();
    let Ok(result) = vfs::read(CONFIG_PATH, 0, 65536) else {
        return config;
    };
    let Ok(text) = String::from_utf8(result.data) else {
        return config;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return config;
    };
    config.endpoint = value
        .get("endpoint")
        .and_then(Value::as_str)
        .filter(|endpoint| endpoint.starts_with("http://") || endpoint.starts_with("https://"))
        .map(|endpoint| endpoint.trim_end_matches('/').to_string());
    config.sse = value
        .get("sse")
        .and_then(Value::as_str)
        .filter(|sse| sse.starts_with("http://") || sse.starts_with("https://"))
        .map(str::to_string);
    config.workspace = value
        .get("workspace")
        .and_then(Value::as_str)
        .filter(|workspace| !workspace.trim().is_empty())
        .map(str::to_string);
    config
}

/// Open the session's update stream, run one prompt, and fold the chunks into
/// the canonical thread. The stream is bounded by the request timeout; ACP
/// leaves it open after `end_turn`, so a short drain is the honest end.
fn stream_prompt(client: &Client, id: &str, text: &str) -> Result<String, types::Error> {
    let stream = http::open(&http::Request {
        url: client.sse_url(id),
        method: http::Method::Get,
        headers: vec![http::Header {
            name: "accept".into(),
            value: "text/event-stream".into(),
        }],
        body: None,
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: 8_000,
    })
    .map_err(map_http)?;
    let response = prompt(client, id, text)?;
    let mut reply = String::new();
    if let Some(content) = response.get("content").and_then(Value::as_array) {
        for part in content {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                reply.push_str(text);
            }
        }
    }
    let (streamed, seq) = read_updates(stream, id)?;
    if !streamed.is_empty() {
        reply = streamed;
    }
    write_thread(id, &reply, seq)?;
    Ok(reply)
}

fn read_updates(stream: http::ResponseStream, id: &str) -> Result<(String, u64), types::Error> {
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut seq = 0u64;
    let mut bytes = 0usize;
    loop {
        match stream.read(65536) {
            Ok(Some(data)) => {
                bytes = bytes.saturating_add(data.len());
                if bytes > MAX_BODY as usize {
                    break;
                }
                buffer.extend(data);
                while let Some(at) = buffer.windows(2).position(|window| window == b"\n\n") {
                    let frame: Vec<u8> = buffer.drain(..at + 2).collect();
                    let frame = String::from_utf8_lossy(&frame);
                    for line in frame.lines() {
                        let Some(data) = line.strip_prefix("data:") else {
                            continue;
                        };
                        let Ok(update) = serde_json::from_str::<Value>(data.trim()) else {
                            continue;
                        };
                        if let Some(chunk) = update
                            .get("content")
                            .and_then(|content| content.get("text"))
                            .and_then(Value::as_str)
                        {
                            text.push_str(chunk);
                            seq += 1;
                            write_event(id, seq, "message", "assistant", chunk)?;
                        } else if let Some(kind) =
                            update.get("sessionUpdate").and_then(Value::as_str)
                        {
                            seq += 1;
                            write_event(id, seq, "update", "system", kind)?;
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    Ok((text, seq))
}

fn write_event(id: &str, seq: u64, kind: &str, role: &str, text: &str) -> Result<(), types::Error> {
    let dir = format!("agents/{id}/events");
    let _ = vfs_mut::mkdir("agents");
    let _ = vfs_mut::mkdir(&format!("agents/{id}"));
    let _ = vfs_mut::mkdir(&dir);
    let event = json!({
        "schema": "fungos.thread.event.v1",
        "id": format!("{seq}-{kind}-0"),
        "seq": seq,
        "at": 0,
        "kind": kind,
        "role": role,
        "harness": "acp",
        "model": null,
        "text": text,
        "tool": null,
        "status": "ok",
    });
    vfs_mut::write(
        &format!("{dir}/{seq}-{kind}-0.json"),
        serde_json::to_vec(&event)
            .map_err(|_| types::Error::Io)?
            .as_slice(),
        "application/json",
        None,
        None,
    )
    .map_err(map_vfs)?;
    Ok(())
}

fn write_thread(id: &str, reply: &str, seq: u64) -> Result<(), types::Error> {
    let _ = vfs_mut::mkdir("agents");
    let _ = vfs_mut::mkdir(&format!("agents/{id}"));
    let path = format!("agents/{id}/thread.json");
    let version = vfs::stat(&path).ok().map(|entry| entry.version);
    let thread = json!({
        "schema": "fungos.thread.v1",
        "id": id,
        "title": if reply.is_empty() { id } else { reply },
        "workspace": null,
        "active": {"harness": "acp", "session": id, "model": null, "since": 0},
        "head": seq,
        "revision": seq + 1,
        "created-at": 0,
        "updated-at": 0,
    });
    vfs_mut::write(
        &path,
        serde_json::to_vec(&thread)
            .map_err(|_| types::Error::Io)?
            .as_slice(),
        "application/json",
        version,
        None,
    )
    .map_err(map_vfs)?;
    Ok(())
}

fn map_vfs(error: vfs_mut::Error) -> types::Error {
    let _ = error;
    types::Error::Io
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
            http::Header {
                name: "content-type".into(),
                value: "application/json".into(),
            },
            http::Header {
                name: "accept".into(),
                value: "application/json".into(),
            },
        ],
        body: Some(body),
        credential: None,
        max_bytes: MAX_BODY,
        timeout_ms: TIMEOUT_MS,
    })
    .map_err(|error| format!("http: {error:?}"))?;
    let text = String::from_utf8(response.body).map_err(|error| error.to_string())?;
    let value: Value =
        serde_json::from_str(text.trim()).map_err(|error| format!("json: {error}"))?;
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

fn map_http(error: http::Error) -> types::Error {
    match error {
        http::Error::Denied => types::Error::Denied,
        http::Error::Invalid => types::Error::Invalid,
        http::Error::OverBudget => types::Error::OverBudget,
        http::Error::Unsupported => types::Error::Unsupported,
        _ => types::Error::Io,
    }
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
