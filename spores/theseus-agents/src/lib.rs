//! Theseus -> soil:agents.
//!
//! Theseus already materializes a turn per event in `theseus:turns`. This
//! adapter reads that book and presents conversations as normalized agents. It
//! does not touch the Theseus image, and it never invents a field the records do
//! not carry.

wit_bindgen::generate!({
    path: "wit",
    world: "theseus-agents",
    generate_all,
});

use serde_json::Value;
use soil::records::records;
use soil::vfs::vfs;

use exports::fungos::theseus_agents::{control, dispatch, metrics, observe};
mod types {
    pub use super::observe::{
        Activity, ActivityKind, Agent, Binding, Device, Error, Escalation, Goal, Phase, Role,
        Scope, ScopeKind, Status, Turn, Usage,
    };
}

const TURNS: &str = "theseus:turns";
const MAX_RECORDS: usize = 20_000;
const MAX_ACTIVITY: usize = 32;
const MAX_TRANSCRIPT: u32 = 200;

struct Component;

export!(Component);

#[derive(Clone, Debug, Default)]
struct Tokens {
    input: u64,
    output: u64,
    context: u64,
    total: u64,
}

#[derive(Clone, Debug, Default)]
struct Call {
    id: String,
    code: String,
    status: String,
    started: u64,
    finished: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct Turn {
    conversation: String,
    task: String,
    reply: String,
    at: u64,
    started: u64,
    phase: String,
    status: String,
    revision: u64,
    tokens: Tokens,
    model_time_ms: u64,
    calls: Vec<Call>,
}

struct Loaded {
    agents: Vec<types::Agent>,
    transcripts: Vec<(String, Vec<types::Turn>)>,
    measured_output_tokens: u64,
    measured_time_ms: u64,
}

fn u64_field(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn trim(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if out.len() + ch.len_utf8() > max {
            break;
        }
        out.push(ch);
    }
    out
}

fn one_line(text: &str, max: usize) -> String {
    trim(&text.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

fn token_count(value: Option<&Value>, names: &[&str]) -> u64 {
    let Some(value) = value else {
        return 0;
    };
    for name in names {
        if let Some(count) = value.get(*name).and_then(Value::as_u64) {
            return count;
        }
    }
    0
}

fn parse_tokens(text: &str) -> Tokens {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Tokens::default();
    };
    let input = token_count(Some(&value), &["prompt_tokens", "input_tokens"]);
    let output = token_count(Some(&value), &["completion_tokens", "output_tokens"]);
    let total = token_count(Some(&value), &["total_tokens"]);
    Tokens {
        input,
        output,
        context: input,
        total: if total == 0 { input + output } else { total },
    }
}

fn parse_call(value: &Value) -> Call {
    Call {
        id: str_field(value, "id").to_string(),
        code: str_field(value, "code").to_string(),
        status: str_field(value, "status").to_string(),
        started: u64_field(value, "started"),
        finished: value.get("finished").and_then(Value::as_u64),
    }
}

fn parse_turn(value: &Value, revision: u64) -> Turn {
    let usage = str_field(value, "usage");
    let calls = value
        .get("calls")
        .and_then(Value::as_array)
        .map(|calls| calls.iter().map(parse_call).collect())
        .unwrap_or_default();
    Turn {
        conversation: str_field(value, "conversation").to_string(),
        task: str_field(value, "task").to_string(),
        reply: str_field(value, "reply").to_string(),
        at: u64_field(value, "at"),
        started: u64_field(value, "started"),
        phase: str_field(value, "phase").to_string(),
        status: str_field(value, "status").to_string(),
        revision,
        tokens: parse_tokens(usage),
        model_time_ms: u64_field(value, "model-time-ms"),
        calls,
    }
}

fn scan_turns() -> Result<Vec<Turn>, types::Error> {
    let mut turns = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = None;
    loop {
        let page = records::scan(TURNS, cursor.as_deref(), 4096).map_err(|_| types::Error::Io)?;
        for entry in page.entries {
            // The loam records host opens one collection over a whole book, so
            // scan() can still return sibling collections. The adapter is the
            // one place that knows the Theseus turn prefix.
            if !entry.key.id.starts_with("theseus:turns:") {
                continue;
            }
            if !seen.insert(entry.key.id.clone()) {
                continue;
            }
            let Ok(value) = serde_json::from_slice::<Value>(&entry.body) else {
                continue;
            };
            let turn = parse_turn(&value, entry.revision);
            if !turn.conversation.is_empty() {
                turns.push(turn);
            }
            if turns.len() >= MAX_RECORDS {
                break;
            }
        }
        if turns.len() >= MAX_RECORDS {
            break;
        }
        match page.next {
            Some(next) if Some(next.as_str()) != cursor.as_deref() => cursor = Some(next),
            _ => break,
        }
    }
    turns.sort_by_key(|turn| (turn.started, turn.at, turn.revision));
    Ok(turns)
}

fn phase(turn: Option<&Turn>) -> types::Phase {
    let Some(turn) = turn else {
        return types::Phase::Idle;
    };
    match turn.phase.as_str() {
        "done" => types::Phase::Finished,
        "stop" => types::Phase::Paused,
        "error" => types::Phase::Failed,
        "tool-start" | "tool-result" | "tool-error" | "tool-cancel" | "result" => {
            types::Phase::Working
        }
        "start" | "reasoning" | "reasoning-delta" | "content-delta" | "code" | "reply"
        | "usage" => types::Phase::Thinking,
        _ if turn.status == "error" => types::Phase::Failed,
        _ if turn.status == "done" => types::Phase::Finished,
        _ => types::Phase::Idle,
    }
}

fn activity_kind(code: &str) -> types::ActivityKind {
    let code = code.to_ascii_lowercase();
    if code.contains("codemode/exec") || code.contains("codemode/background") {
        types::ActivityKind::Shell
    } else if code.contains("codemode/read-file")
        || code.contains("codemode/write-file")
        || code.contains("codemode/list")
        || code.contains("codemode/git")
    {
        types::ActivityKind::Files
    } else if code.contains("http") {
        types::ActivityKind::Network
    } else if code.contains("inference") {
        types::ActivityKind::Model
    } else if code.contains("memory") {
        types::ActivityKind::Memory
    } else if code.contains("skill") {
        types::ActivityKind::Skill
    } else if code.contains("hook") {
        types::ActivityKind::Hook
    } else if code.contains("agent") {
        types::ActivityKind::Agent
    } else {
        types::ActivityKind::Other
    }
}

fn label(code: &str) -> String {
    let first = code
        .trim()
        .trim_start_matches('(')
        .split(|ch: char| ch.is_whitespace() || ch == '(' || ch == ')')
        .next()
        .unwrap_or("tool");
    trim(first, 80)
}

fn status(call: &Call) -> types::Status {
    match call.status.as_str() {
        "done" | "ok" => types::Status::Ok,
        "error" => types::Status::Failed,
        "canceled" | "cancel" => types::Status::Blocked,
        _ => types::Status::Running,
    }
}

fn activity(turns: &[&Turn]) -> Vec<types::Activity> {
    let mut rows = Vec::new();
    for turn in turns.iter().rev() {
        for call in turn.calls.iter().rev() {
            rows.push(types::Activity {
                kind: activity_kind(&call.code),
                label: label(&call.code),
                detail: if call.code.is_empty() {
                    None
                } else {
                    Some(trim(&call.code, 240))
                },
                status: status(call),
                started_at_ms: if call.started == 0 {
                    turn.started
                } else {
                    call.started
                },
                finished_at_ms: call.finished,
            });
            if rows.len() >= MAX_ACTIVITY {
                rows.reverse();
                return rows;
            }
        }
    }
    rows.reverse();
    rows
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

fn agent(conversation: &str, turns: &[Turn]) -> Option<types::Agent> {
    let first = turns.first()?;
    let last = turns.last()?;
    let task = turns
        .iter()
        .find(|turn| !turn.task.trim().is_empty())
        .map(|turn| turn.task.clone())
        .unwrap_or_default();
    let mut usage = types::Usage {
        context_tokens: last.tokens.context,
        context_window: 0,
        input_tokens: 0,
        output_tokens: 0,
        turns: turns
            .iter()
            .filter(|turn| !turn.reply.trim().is_empty())
            .count() as u32,
        tool_calls: 0,
        tool_errors: 0,
        model_time_ms: 0,
    };
    for turn in turns {
        usage.input_tokens += turn.tokens.input;
        usage.output_tokens += turn.tokens.output;
        usage.tool_calls += turn.calls.len() as u64;
        usage.tool_errors += turn
            .calls
            .iter()
            .filter(|call| matches!(call.status.as_str(), "error" | "canceled" | "cancel"))
            .count() as u64;
        usage.model_time_ms += turn.model_time_ms;
    }
    let title = if task.is_empty() {
        format!("Theseus {conversation}")
    } else {
        one_line(&task, 80)
    };
    let activity = activity(&turns.iter().collect::<Vec<_>>());
    let phase = phase(Some(last));
    Some(types::Agent {
        id: format!("theseus:{conversation}"),
        harness: "theseus".to_string(),
        title,
        workspace: None,
        model: None,
        device: None,
        avatar: None,
        phase,
        goal: if task.is_empty() {
            None
        } else {
            Some(types::Goal {
                text: task,
                set_at_ms: first.started,
            })
        },
        owner: None,
        activity,
        prompts: Vec::new(),
        usage,
        revision: last.revision,
        started_at_ms: first.started,
        updated_at_ms: last.at.max(first.started),
    })
}

fn load() -> Result<Loaded, types::Error> {
    let turns = scan_turns()?;
    let mut conversations: Vec<String> =
        turns.iter().map(|turn| turn.conversation.clone()).collect();
    conversations.sort();
    conversations.dedup();
    let mut agents = Vec::new();
    let mut transcripts = Vec::new();
    let mut measured_output_tokens = 0u64;
    let mut measured_time_ms = 0u64;
    for conversation in conversations {
        let turns: Vec<Turn> = turns
            .iter()
            .filter(|turn| turn.conversation == conversation)
            .cloned()
            .collect();
        if let Some(agent) = agent(&conversation, &turns) {
            agents.push(agent);
        }
        for turn in &turns {
            if turn.model_time_ms > 0 {
                measured_output_tokens += turn.tokens.output;
                measured_time_ms += turn.model_time_ms;
            }
        }
        let mut rows = Vec::new();
        for turn in &turns {
            if !turn.task.trim().is_empty() {
                rows.push(types::Turn {
                    role: types::Role::User,
                    text: turn.task.clone(),
                    at_ms: turn.started,
                });
            }
            if !turn.reply.trim().is_empty() {
                rows.push(types::Turn {
                    role: types::Role::Assistant,
                    text: turn.reply.clone(),
                    at_ms: turn.at,
                });
            }
        }
        rows.sort_by_key(|turn| turn.at_ms);
        transcripts.push((format!("theseus:{conversation}"), rows));
    }
    agents.sort_by_key(|agent| {
        (
            agent.phase != types::Phase::NeedsYou,
            u64::MAX - agent.updated_at_ms,
        )
    });
    Ok(Loaded {
        agents,
        transcripts,
        measured_output_tokens,
        measured_time_ms,
    })
}

fn event_seq(name: &str) -> u64 {
    name.split('-')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

fn event_turn(value: &Value) -> Option<(u64, types::Turn)> {
    let kind = str_field(value, "kind");
    let seq = u64_field(value, "seq");
    let at = u64_field(value, "at");
    let text = str_field(value, "text").to_string();
    let role = match kind {
        "message" => match str_field(value, "role") {
            "user" => types::Role::User,
            _ => types::Role::Assistant,
        },
        "tool-call" | "tool-result" => types::Role::Tool,
        "notice" | "handoff" => types::Role::Assistant,
        _ => return None,
    };
    Some((
        seq,
        types::Turn {
            role,
            text,
            at_ms: at,
        },
    ))
}

fn transcript_vfs(
    conversation: &str,
    before: Option<u64>,
    limit: u32,
) -> Result<Option<observe::Page>, types::Error> {
    let parent = format!("agents/{conversation}/events");
    let mut entries = Vec::new();
    let mut offset = 0u64;
    loop {
        let page = match vfs::ls(&parent, offset, 512) {
            Ok(page) => page,
            Err(vfs::Error::NotFound | vfs::Error::NotDirectory) => return Ok(None),
            Err(_) => return Err(types::Error::Io),
        };
        entries.extend(page.entries);
        match page.next {
            Some(next) if next != offset => offset = next,
            _ => break,
        }
        if entries.len() > 4096 {
            break;
        }
    }
    entries.sort_by_key(|entry| event_seq(&entry.name));
    let mut events = Vec::new();
    for entry in entries {
        if !entry.name.ends_with(".json") {
            continue;
        }
        let chunk = vfs::read(&entry.path, 0, 1_048_576)
            .map_err(|_| types::Error::Io)?;
        let Ok(value) = serde_json::from_slice::<Value>(&chunk.data) else {
            continue;
        };
        if let Some((seq, turn)) = event_turn(&value) {
            if before.is_none_or(|before| seq < before) {
                events.push((seq, turn));
            }
        }
    }
    events.sort_by_key(|(seq, _)| *seq);
    let take = limit.clamp(1, MAX_TRANSCRIPT) as usize;
    let start = events.len().saturating_sub(take);
    let page = events[start..].to_vec();
    let before = page.first().map(|(seq, _)| *seq);
    Ok(Some(observe::Page {
        turns: page.into_iter().map(|(_, turn)| turn).collect(),
        before,
    }))
}

fn find(loaded: &Loaded, id: &str) -> Option<types::Agent> {
    loaded.agents.iter().find(|agent| agent.id == id).cloned()
}

fn metrics(loaded: &Loaded) -> metrics::Report {
    let mut report = metrics::Report {
        since_ms: 0,
        until_ms: 0,
        agents: loaded.agents.len() as u32,
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
    };
    for agent in &loaded.agents {
        if matches!(agent.phase, types::Phase::Thinking | types::Phase::Working) {
            report.active += 1;
        }
        if agent.phase == types::Phase::NeedsYou {
            report.needs_you += 1;
        }
        report.input_tokens += agent.usage.input_tokens;
        report.output_tokens += agent.usage.output_tokens;
        report.turns += agent.usage.turns;
        report.tool_calls += agent.usage.tool_calls;
        report.tool_errors += agent.usage.tool_errors;
        if agent.usage.model_time_ms > 0 {
            report.measured_agents += 1;
        }
        report.since_ms = if report.since_ms == 0 {
            agent.started_at_ms
        } else {
            report.since_ms.min(agent.started_at_ms)
        };
        report.until_ms = report.until_ms.max(agent.updated_at_ms);
    }
    if loaded.measured_time_ms > 0 {
        report.tokens_per_second_milli =
            loaded.measured_output_tokens * 1000 / loaded.measured_time_ms;
    }
    report.by_harness.push(metrics::HarnessMetrics {
        harness: "theseus".to_string(),
        agents: report.agents,
        input_tokens: report.input_tokens,
        output_tokens: report.output_tokens,
        tool_calls: report.tool_calls,
        tool_errors: report.tool_errors,
        tokens_per_second_milli: report.tokens_per_second_milli,
        measured_agents: report.measured_agents,
    });
    report
}

fn device_scope() -> types::Scope {
    types::Scope {
        kind: types::ScopeKind::Device,
        id: None,
        label: Some("this device".to_string()),
        owner: None,
    }
}

impl observe::Guest for Component {
    fn agents() -> Result<Vec<types::Agent>, types::Error> {
        Ok(load()?.agents)
    }

    fn escalations() -> Result<Vec<types::Escalation>, types::Error> {
        Ok(Vec::new())
    }

    fn devices() -> Vec<types::Device> {
        Vec::new()
    }

    fn models() -> Vec<String> {
        Vec::new()
    }

    fn get(id: String) -> Result<types::Agent, types::Error> {
        find(&load()?, &id).ok_or(types::Error::NotFound)
    }

    fn intent() -> types::Binding {
        types::Binding {
            agent: None,
            view: None,
        }
    }

    fn current_scope() -> types::Scope {
        device_scope()
    }

    fn transcript(
        id: String,
        before: Option<u64>,
        limit: u32,
    ) -> Result<observe::Page, types::Error> {
        if let Some(conversation) = id.strip_prefix("theseus:") {
            if let Some(page) = transcript_vfs(conversation, before, limit)? {
                return Ok(page);
            }
        }
        // Older conversations predate the VFS thread store; keep their turn
        // records readable until the adapter can migrate them.
        let loaded = load()?;
        let rows = loaded
            .transcripts
            .iter()
            .find(|(conversation, _)| *conversation == id)
            .map(|(_, rows)| rows.as_slice())
            .ok_or(types::Error::NotFound)?;
        let mut page: Vec<types::Turn> = rows
            .iter()
            .filter(|turn| before.is_none_or(|before| turn.at_ms < before))
            .rev()
            .take(limit.clamp(1, MAX_TRANSCRIPT) as usize)
            .cloned()
            .collect();
        page.reverse();
        let next = page.first().map(|turn| turn.at_ms);
        Ok(observe::Page {
            turns: page,
            before: next,
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

    fn send(_id: String, _text: String) -> Result<(), control::Error> {
        Err(control::Error::Unsupported)
    }

    fn interrupt(_id: String) -> Result<(), control::Error> {
        Err(control::Error::Unsupported)
    }
}

fn metrics_error(value: types::Error) -> metrics::Error {
    match value {
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

impl metrics::Guest for Component {
    fn stats(
        since_ms: Option<u64>,
        until_ms: Option<u64>,
    ) -> Result<metrics::Report, metrics::Error> {
        let mut report = metrics(&load().map_err(metrics_error)?);
        if let Some(since) = since_ms {
            report.since_ms = report.since_ms.max(since);
        }
        if let Some(until) = until_ms {
            report.until_ms = if report.until_ms == 0 {
                until
            } else {
                report.until_ms.min(until)
            };
        }
        Ok(report)
    }
}

impl dispatch::Guest for Component {
    fn dispatch(_task: dispatch::Task) -> Result<dispatch::Started, dispatch::Error> {
        Err(dispatch::Error::Unsupported)
    }
}

impl exports::fungos::theseus_agents::cli::Guest for Component {
    fn run(args: Vec<String>) -> Result<String, String> {
        let loaded = load().map_err(|error| format!("{error:?}"))?;
        let action = args.first().map(String::as_str).unwrap_or("agents");
        let value = match action {
            "agents" => serde_json::to_value(
                loaded
                    .agents
                    .iter()
                    .map(|agent| {
                        serde_json::json!({
                            "id": agent.id,
                            "title": agent.title,
                            "phase": phase_name(agent.phase),
                            "usage": {
                                "input": agent.usage.input_tokens,
                                "output": agent.usage.output_tokens,
                                "turns": agent.usage.turns,
                                "toolCalls": agent.usage.tool_calls,
                                "toolErrors": agent.usage.tool_errors,
                            },
                            "activity": agent.activity.len(),
                        })
                    })
                    .collect::<Vec<_>>(),
            ),
            "metrics" => {
                let report = metrics(&loaded);
                Ok(serde_json::json!({
                    "agents": report.agents,
                    "active": report.active,
                    "needsYou": report.needs_you,
                    "inputTokens": report.input_tokens,
                    "outputTokens": report.output_tokens,
                    "turns": report.turns,
                    "toolCalls": report.tool_calls,
                    "toolErrors": report.tool_errors,
                    "tokensPerSecondMilli": report.tokens_per_second_milli,
                    "measuredAgents": report.measured_agents,
                    "sinceMs": report.since_ms,
                    "untilMs": report.until_ms,
                }))
            }
            "transcript" => {
                let id = args.get(1).ok_or("transcript needs an agent id")?;
                let vfs_rows = id
                    .strip_prefix("theseus:")
                    .and_then(|conversation| transcript_vfs(conversation, None, 200).ok().flatten())
                    .map(|page| page.turns)
                    .filter(|rows| !rows.is_empty());
                let rows = match vfs_rows {
                    Some(rows) => rows,
                    None => loaded
                        .transcripts
                        .iter()
                        .find(|(conversation, _)| conversation == id)
                        .map(|(_, rows)| rows.clone())
                        .ok_or("unknown agent")?,
                };
                Ok(serde_json::to_value(
                    rows.iter()
                        .map(|turn| {
                            serde_json::json!({
                                "role": match turn.role {
                                    types::Role::User => "user",
                                    types::Role::Assistant => "assistant",
                                    types::Role::Tool => "tool",
                                },
                                "text": turn.text,
                                "atMs": turn.at_ms,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| error.to_string())?)
            }
            other => return Err(format!("unknown action {other:?}")),
        }
        .map_err(|error| error.to_string())?;
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
    }
}
