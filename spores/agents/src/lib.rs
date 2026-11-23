//! One `soil:agents` surface over adapter Spores.
//!
//! The aggregator owns routing and projection, not harness knowledge. Today it
//! composes Theseus; ACP and the bridges plug into the same interfaces later.

wit_bindgen::generate!({
    path: "wit",
    world: "agents",
    generate_all,
});

use exports::fungos::agents::cli;
use exports::soil::agents::types as p;
use exports::soil::agents::{control, dispatch, metrics, observe};
use fungos::theseus_agents::observe as t;
use fungos::theseus_agents::{cli as t_cli, control as t_control, dispatch as t_dispatch};
use fungos::theseus_agents::{metrics as t_metrics, observe as t_observe};

struct Component;

export!(Component);

fn error(value: t::Error) -> p::Error {
    match value {
        t::Error::Denied => p::Error::Denied,
        t::Error::NotFound => p::Error::NotFound,
        t::Error::NotReady => p::Error::NotReady,
        t::Error::Invalid => p::Error::Invalid,
        t::Error::OverBudget => p::Error::OverBudget,
        t::Error::Stale => p::Error::Stale,
        t::Error::Unsupported => p::Error::Unsupported,
        t::Error::Io => p::Error::Io,
    }
}

fn error_control(value: t_control::Error) -> p::Error {
    match value {
        t_control::Error::Denied => p::Error::Denied,
        t_control::Error::NotFound => p::Error::NotFound,
        t_control::Error::NotReady => p::Error::NotReady,
        t_control::Error::Invalid => p::Error::Invalid,
        t_control::Error::OverBudget => p::Error::OverBudget,
        t_control::Error::Stale => p::Error::Stale,
        t_control::Error::Unsupported => p::Error::Unsupported,
        t_control::Error::Io => p::Error::Io,
    }
}

fn error_metrics(value: t_metrics::Error) -> p::Error {
    match value {
        t_metrics::Error::Denied => p::Error::Denied,
        t_metrics::Error::NotFound => p::Error::NotFound,
        t_metrics::Error::NotReady => p::Error::NotReady,
        t_metrics::Error::Invalid => p::Error::Invalid,
        t_metrics::Error::OverBudget => p::Error::OverBudget,
        t_metrics::Error::Stale => p::Error::Stale,
        t_metrics::Error::Unsupported => p::Error::Unsupported,
        t_metrics::Error::Io => p::Error::Io,
    }
}

fn error_dispatch(value: t_dispatch::Error) -> p::Error {
    match value {
        t_dispatch::Error::Denied => p::Error::Denied,
        t_dispatch::Error::NotFound => p::Error::NotFound,
        t_dispatch::Error::NotReady => p::Error::NotReady,
        t_dispatch::Error::Invalid => p::Error::Invalid,
        t_dispatch::Error::OverBudget => p::Error::OverBudget,
        t_dispatch::Error::Stale => p::Error::Stale,
        t_dispatch::Error::Unsupported => p::Error::Unsupported,
        t_dispatch::Error::Io => p::Error::Io,
    }
}

fn phase(value: t::Phase) -> p::Phase {
    match value {
        t::Phase::Connecting => p::Phase::Connecting,
        t::Phase::Idle => p::Phase::Idle,
        t::Phase::Thinking => p::Phase::Thinking,
        t::Phase::Working => p::Phase::Working,
        t::Phase::NeedsYou => p::Phase::NeedsYou,
        t::Phase::Finished => p::Phase::Finished,
        t::Phase::Failed => p::Phase::Failed,
        t::Phase::Paused => p::Phase::Paused,
    }
}

fn activity_kind(value: t::ActivityKind) -> p::ActivityKind {
    match value {
        t::ActivityKind::You => p::ActivityKind::You,
        t::ActivityKind::Model => p::ActivityKind::Model,
        t::ActivityKind::Memory => p::ActivityKind::Memory,
        t::ActivityKind::Files => p::ActivityKind::Files,
        t::ActivityKind::Shell => p::ActivityKind::Shell,
        t::ActivityKind::Network => p::ActivityKind::Network,
        t::ActivityKind::Browser => p::ActivityKind::Browser,
        t::ActivityKind::Agent => p::ActivityKind::Agent,
        t::ActivityKind::Skill => p::ActivityKind::Skill,
        t::ActivityKind::Hook => p::ActivityKind::Hook,
        t::ActivityKind::Other => p::ActivityKind::Other,
    }
}

fn status(value: t::Status) -> p::Status {
    match value {
        t::Status::Running => p::Status::Running,
        t::Status::Ok => p::Status::Ok,
        t::Status::Failed => p::Status::Failed,
        t::Status::Blocked => p::Status::Blocked,
    }
}

fn prompt_kind(value: t::PromptKind) -> p::PromptKind {
    match value {
        t::PromptKind::Question => p::PromptKind::Question,
        t::PromptKind::Approval => p::PromptKind::Approval,
        t::PromptKind::Choice => p::PromptKind::Choice,
    }
}

fn role(value: t::Role) -> p::Role {
    match value {
        t::Role::User => p::Role::User,
        t::Role::Assistant => p::Role::Assistant,
        t::Role::Tool => p::Role::Tool,
    }
}

fn scope_kind(value: t::ScopeKind) -> p::ScopeKind {
    match value {
        t::ScopeKind::Device => p::ScopeKind::Device,
        t::ScopeKind::Desktop => p::ScopeKind::Desktop,
        t::ScopeKind::Principal => p::ScopeKind::Principal,
    }
}

fn avatar_kind(value: t::AvatarKind) -> p::AvatarKind {
    match value {
        t::AvatarKind::Image => p::AvatarKind::Image,
        t::AvatarKind::Animation => p::AvatarKind::Animation,
        t::AvatarKind::Glyph => p::AvatarKind::Glyph,
    }
}

fn owner(value: t::Owner) -> p::Owner {
    p::Owner {
        actor: value.actor,
        display_name: value.display_name,
    }
}

fn avatar(value: t::Avatar) -> p::Avatar {
    p::Avatar {
        kind: avatar_kind(value.kind),
        asset: value.asset,
        mime: value.mime,
        glyph: value.glyph,
        alt: value.alt,
        animated: value.animated,
    }
}

fn activity(value: t::Activity) -> p::Activity {
    p::Activity {
        kind: activity_kind(value.kind),
        label: value.label,
        detail: value.detail,
        status: status(value.status),
        started_at_ms: value.started_at_ms,
        finished_at_ms: value.finished_at_ms,
    }
}

fn choice(value: t::Choice) -> p::Choice {
    p::Choice {
        id: value.id,
        label: value.label,
        detail: value.detail,
        danger: value.danger,
    }
}

fn prompt(value: t::Prompt) -> p::Prompt {
    p::Prompt {
        id: value.id,
        kind: prompt_kind(value.kind),
        title: value.title,
        body: value.body,
        details: value.details,
        origin: value.origin,
        options: value.options.into_iter().map(choice).collect(),
        allows_text: value.allows_text,
        asked_at_ms: value.asked_at_ms,
        deadline_at_ms: value.deadline_at_ms,
    }
}

fn usage(value: t::Usage) -> p::Usage {
    p::Usage {
        context_tokens: value.context_tokens,
        context_window: value.context_window,
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        turns: value.turns,
        tool_calls: value.tool_calls,
        tool_errors: value.tool_errors,
        model_time_ms: value.model_time_ms,
    }
}

fn goal(value: t::Goal) -> p::Goal {
    p::Goal {
        text: value.text,
        set_at_ms: value.set_at_ms,
    }
}

fn agent(value: t::Agent) -> p::Agent {
    p::Agent {
        id: value.id,
        harness: value.harness,
        title: value.title,
        workspace: value.workspace,
        model: value.model,
        device: value.device,
        avatar: value.avatar.map(avatar),
        phase: phase(value.phase),
        goal: value.goal.map(goal),
        owner: value.owner.map(owner),
        activity: value.activity.into_iter().map(activity).collect(),
        prompts: value.prompts.into_iter().map(prompt).collect(),
        usage: usage(value.usage),
        revision: value.revision,
        started_at_ms: value.started_at_ms,
        updated_at_ms: value.updated_at_ms,
    }
}

fn turn(value: t::Turn) -> p::Turn {
    p::Turn {
        role: role(value.role),
        text: value.text,
        at_ms: value.at_ms,
    }
}

fn escalation(value: t::Escalation) -> p::Escalation {
    p::Escalation {
        agent_id: value.agent_id,
        agent_title: value.agent_title,
        prompt: prompt(value.prompt),
    }
}

fn device(value: t::Device) -> p::Device {
    p::Device {
        id: value.id,
        label: value.label,
        current: value.current,
        attached: value.attached,
        detail: value.detail,
    }
}

fn scope(value: t::Scope) -> p::Scope {
    p::Scope {
        kind: scope_kind(value.kind),
        id: value.id,
        label: value.label,
        owner: value.owner.map(owner),
    }
}

fn binding(value: t::Binding) -> p::Binding {
    p::Binding {
        agent: value.agent,
        view: value.view,
    }
}

fn response(value: p::Response) -> t_control::Response {
    t_control::Response {
        choice: value.choice,
        text: value.text,
    }
}

impl observe::Guest for Component {
    fn agents() -> Result<Vec<p::Agent>, p::Error> {
        Ok(t_observe::agents()
            .map_err(error)?
            .into_iter()
            .map(agent)
            .collect())
    }

    fn escalations() -> Result<Vec<p::Escalation>, p::Error> {
        Ok(t_observe::escalations()
            .map_err(error)?
            .into_iter()
            .map(escalation)
            .collect())
    }

    fn devices() -> Vec<p::Device> {
        t_observe::devices().into_iter().map(device).collect()
    }

    fn models() -> Vec<String> {
        t_observe::models()
    }

    fn get(id: String) -> Result<p::Agent, p::Error> {
        t_observe::get(&id).map_err(error).map(agent)
    }

    fn intent() -> p::Binding {
        binding(t_observe::intent())
    }

    fn current_scope() -> p::Scope {
        scope(t_observe::current_scope())
    }

    fn transcript(id: String, before: Option<u64>, limit: u32) -> Result<observe::Page, p::Error> {
        let page = t_observe::transcript(&id, before, limit).map_err(error)?;
        Ok(observe::Page {
            turns: page.turns.into_iter().map(turn).collect(),
            before: page.before,
        })
    }
}

impl control::Guest for Component {
    fn respond(id: String, prompt_id: String, answer: p::Response) -> Result<(), p::Error> {
        t_control::respond(&id, &prompt_id, &response(answer)).map_err(error_control)
    }

    fn send(id: String, text: String) -> Result<(), p::Error> {
        t_control::send(&id, &text).map_err(error_control)
    }

    fn interrupt(id: String) -> Result<(), p::Error> {
        t_control::interrupt(&id).map_err(error_control)
    }
}

fn report(value: t_metrics::Report) -> metrics::Report {
    metrics::Report {
        since_ms: value.since_ms,
        until_ms: value.until_ms,
        agents: value.agents,
        active: value.active,
        needs_you: value.needs_you,
        input_tokens: value.input_tokens,
        output_tokens: value.output_tokens,
        turns: value.turns,
        tool_calls: value.tool_calls,
        tool_errors: value.tool_errors,
        tokens_per_second_milli: value.tokens_per_second_milli,
        measured_agents: value.measured_agents,
        by_harness: value
            .by_harness
            .into_iter()
            .map(|row| metrics::HarnessMetrics {
                harness: row.harness,
                agents: row.agents,
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                tool_calls: row.tool_calls,
                tool_errors: row.tool_errors,
                tokens_per_second_milli: row.tokens_per_second_milli,
                measured_agents: row.measured_agents,
            })
            .collect(),
    }
}

impl metrics::Guest for Component {
    fn stats(since_ms: Option<u64>, until_ms: Option<u64>) -> Result<metrics::Report, p::Error> {
        t_metrics::stats(since_ms, until_ms)
            .map_err(error_metrics)
            .map(report)
    }
}

fn to_task(task: dispatch::Task) -> t_dispatch::Task {
    t_dispatch::Task {
        text: task.text,
        harness: task.harness,
        environment: task.environment,
        title: task.title,
        model: task.model,
    }
}

impl dispatch::Guest for Component {
    fn dispatch(task: dispatch::Task) -> Result<dispatch::Started, p::Error> {
        let started = t_dispatch::dispatch(&to_task(task)).map_err(error_dispatch)?;
        Ok(dispatch::Started {
            agent_id: started.agent_id,
            conversation: started.conversation,
        })
    }
}

impl cli::Guest for Component {
    fn run(args: Vec<String>) -> Result<String, String> {
        t_cli::run(&args)
    }
}
