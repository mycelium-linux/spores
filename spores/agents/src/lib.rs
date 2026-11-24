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
use fungos::acp_agents::observe as a;
use fungos::acp_agents::{control as a_control, dispatch as a_dispatch};
use fungos::acp_agents::{metrics as a_metrics, observe as a_observe};
use fungos::theseus_agents::observe as t;
use fungos::theseus_agents::{cli as t_cli, control as t_control, dispatch as t_dispatch};
use fungos::theseus_agents::{metrics as t_metrics, observe as t_observe};

struct Component;

export!(Component);

impl exports::wasi::cli::run::Guest for Component {
    fn run() -> Result<(), ()> {
        let args = wasi::cli::environment::get_arguments();
        let args = if args.is_empty() {
            vec!["agents".to_string()]
        } else {
            args.into_iter().skip(1).collect()
        };
        match <Component as cli::Guest>::run(args) {
            Ok(text) => {
                println!("{text}");
                Ok(())
            }
            Err(error) => {
                eprintln!("{error}");
                Err(())
            }
        }
    }
}

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

fn a_error(value: a::Error) -> p::Error {
    match value {
        a::Error::Denied => p::Error::Denied,
        a::Error::NotFound => p::Error::NotFound,
        a::Error::NotReady => p::Error::NotReady,
        a::Error::Invalid => p::Error::Invalid,
        a::Error::OverBudget => p::Error::OverBudget,
        a::Error::Stale => p::Error::Stale,
        a::Error::Unsupported => p::Error::Unsupported,
        a::Error::Io => p::Error::Io,
    }
}

fn a_error_control(value: a_control::Error) -> p::Error {
    match value {
        a_control::Error::Denied => p::Error::Denied,
        a_control::Error::NotFound => p::Error::NotFound,
        a_control::Error::NotReady => p::Error::NotReady,
        a_control::Error::Invalid => p::Error::Invalid,
        a_control::Error::OverBudget => p::Error::OverBudget,
        a_control::Error::Stale => p::Error::Stale,
        a_control::Error::Unsupported => p::Error::Unsupported,
        a_control::Error::Io => p::Error::Io,
    }
}

fn a_error_metrics(value: a_metrics::Error) -> p::Error {
    match value {
        a_metrics::Error::Denied => p::Error::Denied,
        a_metrics::Error::NotFound => p::Error::NotFound,
        a_metrics::Error::NotReady => p::Error::NotReady,
        a_metrics::Error::Invalid => p::Error::Invalid,
        a_metrics::Error::OverBudget => p::Error::OverBudget,
        a_metrics::Error::Stale => p::Error::Stale,
        a_metrics::Error::Unsupported => p::Error::Unsupported,
        a_metrics::Error::Io => p::Error::Io,
    }
}

fn a_error_dispatch(value: a_dispatch::Error) -> p::Error {
    match value {
        a_dispatch::Error::Denied => p::Error::Denied,
        a_dispatch::Error::NotFound => p::Error::NotFound,
        a_dispatch::Error::NotReady => p::Error::NotReady,
        a_dispatch::Error::Invalid => p::Error::Invalid,
        a_dispatch::Error::OverBudget => p::Error::OverBudget,
        a_dispatch::Error::Stale => p::Error::Stale,
        a_dispatch::Error::Unsupported => p::Error::Unsupported,
        a_dispatch::Error::Io => p::Error::Io,
    }
}

fn a_phase(value: a::Phase) -> p::Phase {
    match value {
        a::Phase::Connecting => p::Phase::Connecting,
        a::Phase::Idle => p::Phase::Idle,
        a::Phase::Thinking => p::Phase::Thinking,
        a::Phase::Working => p::Phase::Working,
        a::Phase::NeedsYou => p::Phase::NeedsYou,
        a::Phase::Finished => p::Phase::Finished,
        a::Phase::Failed => p::Phase::Failed,
        a::Phase::Paused => p::Phase::Paused,
    }
}

fn a_activity_kind(value: a::ActivityKind) -> p::ActivityKind {
    match value {
        a::ActivityKind::You => p::ActivityKind::You,
        a::ActivityKind::Model => p::ActivityKind::Model,
        a::ActivityKind::Memory => p::ActivityKind::Memory,
        a::ActivityKind::Files => p::ActivityKind::Files,
        a::ActivityKind::Shell => p::ActivityKind::Shell,
        a::ActivityKind::Network => p::ActivityKind::Network,
        a::ActivityKind::Browser => p::ActivityKind::Browser,
        a::ActivityKind::Agent => p::ActivityKind::Agent,
        a::ActivityKind::Skill => p::ActivityKind::Skill,
        a::ActivityKind::Hook => p::ActivityKind::Hook,
        a::ActivityKind::Other => p::ActivityKind::Other,
    }
}

fn a_status(value: a::Status) -> p::Status {
    match value {
        a::Status::Running => p::Status::Running,
        a::Status::Ok => p::Status::Ok,
        a::Status::Failed => p::Status::Failed,
        a::Status::Blocked => p::Status::Blocked,
    }
}

fn a_prompt_kind(value: a::PromptKind) -> p::PromptKind {
    match value {
        a::PromptKind::Question => p::PromptKind::Question,
        a::PromptKind::Approval => p::PromptKind::Approval,
        a::PromptKind::Choice => p::PromptKind::Choice,
    }
}

fn a_role(value: a::Role) -> p::Role {
    match value {
        a::Role::User => p::Role::User,
        a::Role::Assistant => p::Role::Assistant,
        a::Role::Tool => p::Role::Tool,
    }
}

fn a_scope_kind(value: a::ScopeKind) -> p::ScopeKind {
    match value {
        a::ScopeKind::Device => p::ScopeKind::Device,
        a::ScopeKind::Desktop => p::ScopeKind::Desktop,
        a::ScopeKind::Principal => p::ScopeKind::Principal,
    }
}

fn a_avatar_kind(value: a::AvatarKind) -> p::AvatarKind {
    match value {
        a::AvatarKind::Image => p::AvatarKind::Image,
        a::AvatarKind::Animation => p::AvatarKind::Animation,
        a::AvatarKind::Glyph => p::AvatarKind::Glyph,
    }
}

fn a_owner(value: a::Owner) -> p::Owner {
    p::Owner {
        actor: value.actor,
        display_name: value.display_name,
    }
}

fn a_avatar(value: a::Avatar) -> p::Avatar {
    p::Avatar {
        kind: a_avatar_kind(value.kind),
        asset: value.asset,
        mime: value.mime,
        glyph: value.glyph,
        alt: value.alt,
        animated: value.animated,
    }
}

fn a_activity(value: a::Activity) -> p::Activity {
    p::Activity {
        kind: a_activity_kind(value.kind),
        label: value.label,
        detail: value.detail,
        status: a_status(value.status),
        started_at_ms: value.started_at_ms,
        finished_at_ms: value.finished_at_ms,
    }
}

fn a_choice(value: a::Choice) -> p::Choice {
    p::Choice {
        id: value.id,
        label: value.label,
        detail: value.detail,
        danger: value.danger,
    }
}

fn a_prompt(value: a::Prompt) -> p::Prompt {
    p::Prompt {
        id: value.id,
        kind: a_prompt_kind(value.kind),
        title: value.title,
        body: value.body,
        details: value.details,
        origin: value.origin,
        options: value.options.into_iter().map(a_choice).collect(),
        allows_text: value.allows_text,
        asked_at_ms: value.asked_at_ms,
        deadline_at_ms: value.deadline_at_ms,
    }
}

fn a_usage(value: a::Usage) -> p::Usage {
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

fn a_goal(value: a::Goal) -> p::Goal {
    p::Goal {
        text: value.text,
        set_at_ms: value.set_at_ms,
    }
}

fn a_agent(value: a::Agent) -> p::Agent {
    p::Agent {
        id: value.id,
        harness: value.harness,
        title: value.title,
        workspace: value.workspace,
        model: value.model,
        device: value.device,
        avatar: value.avatar.map(a_avatar),
        phase: a_phase(value.phase),
        goal: value.goal.map(a_goal),
        owner: value.owner.map(a_owner),
        activity: value.activity.into_iter().map(a_activity).collect(),
        prompts: value.prompts.into_iter().map(a_prompt).collect(),
        usage: a_usage(value.usage),
        revision: value.revision,
        started_at_ms: value.started_at_ms,
        updated_at_ms: value.updated_at_ms,
    }
}

fn a_turn(value: a::Turn) -> p::Turn {
    p::Turn {
        role: a_role(value.role),
        text: value.text,
        at_ms: value.at_ms,
    }
}

fn a_escalation(value: a::Escalation) -> p::Escalation {
    p::Escalation {
        agent_id: value.agent_id,
        agent_title: value.agent_title,
        prompt: a_prompt(value.prompt),
    }
}

fn a_device(value: a::Device) -> p::Device {
    p::Device {
        id: value.id,
        label: value.label,
        current: value.current,
        attached: value.attached,
        detail: value.detail,
    }
}

fn a_scope(value: a::Scope) -> p::Scope {
    p::Scope {
        kind: a_scope_kind(value.kind),
        id: value.id,
        label: value.label,
        owner: value.owner.map(a_owner),
    }
}

fn a_binding(value: a::Binding) -> p::Binding {
    p::Binding {
        agent: value.agent,
        view: value.view,
    }
}

fn a_response(value: p::Response) -> a_control::Response {
    a_control::Response {
        choice: value.choice,
        text: value.text,
    }
}

fn a_report(value: a_metrics::Report) -> metrics::Report {
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

fn a_to_task(task: dispatch::Task) -> a_dispatch::Task {
    a_dispatch::Task {
        text: task.text,
        harness: task.harness,
        environment: task.environment,
        title: task.title,
        model: task.model,
    }
}

impl observe::Guest for Component {
    fn agents() -> Result<Vec<p::Agent>, p::Error> {
        let mut agents: Vec<p::Agent> = t_observe::agents()
            .map_err(error)?
            .into_iter()
            .map(agent)
            .collect();
        // One harness being down must not empty the wall. The adapter is the
        // only thing that failed; the other roster is still true.
        if let Ok(list) = a_observe::agents() {
            agents.extend(list.into_iter().map(a_agent));
        }
        Ok(agents)
    }

    fn escalations() -> Result<Vec<p::Escalation>, p::Error> {
        let mut escalations: Vec<p::Escalation> = t_observe::escalations()
            .map_err(error)?
            .into_iter()
            .map(escalation)
            .collect();
        if let Ok(list) = a_observe::escalations() {
            escalations.extend(list.into_iter().map(a_escalation));
        }
        Ok(escalations)
    }

    fn devices() -> Vec<p::Device> {
        let mut devices: Vec<p::Device> = t_observe::devices().into_iter().map(device).collect();
        devices.extend(a_observe::devices().into_iter().map(a_device));
        devices
    }

    fn models() -> Vec<String> {
        let mut models = t_observe::models();
        for model in a_observe::models() {
            if !models.contains(&model) {
                models.push(model);
            }
        }
        models
    }

    fn get(id: String) -> Result<p::Agent, p::Error> {
        match t_observe::get(&id) {
            Ok(value) => Ok(agent(value)),
            Err(t::Error::NotFound) => a_observe::get(&id).map_err(a_error).map(a_agent),
            Err(failure) => Err(error(failure)),
        }
    }

    fn intent() -> p::Binding {
        binding(t_observe::intent())
    }

    fn current_scope() -> p::Scope {
        scope(t_observe::current_scope())
    }

    fn transcript(id: String, before: Option<u64>, limit: u32) -> Result<observe::Page, p::Error> {
        match t_observe::transcript(&id, before, limit) {
            Ok(page) => Ok(observe::Page {
                turns: page.turns.into_iter().map(turn).collect(),
                before: page.before,
            }),
            Err(t::Error::NotFound) => {
                let page = a_observe::transcript(&id, before, limit).map_err(a_error)?;
                Ok(observe::Page {
                    turns: page.turns.into_iter().map(a_turn).collect(),
                    before: page.before,
                })
            }
            Err(failure) => Err(error(failure)),
        }
    }
}

impl control::Guest for Component {
    fn respond(id: String, prompt_id: String, answer: p::Response) -> Result<(), p::Error> {
        match t_control::respond(&id, &prompt_id, &response(answer.clone())) {
            Ok(()) => Ok(()),
            Err(t_control::Error::NotFound) => {
                a_control::respond(&id, &prompt_id, &a_response(answer)).map_err(a_error_control)
            }
            Err(failure) => Err(error_control(failure)),
        }
    }

    fn send(id: String, text: String) -> Result<(), p::Error> {
        match t_control::send(&id, &text) {
            Ok(()) => Ok(()),
            Err(t_control::Error::NotFound) => a_control::send(&id, &text).map_err(a_error_control),
            Err(failure) => Err(error_control(failure)),
        }
    }

    fn interrupt(id: String) -> Result<(), p::Error> {
        match t_control::interrupt(&id) {
            Ok(()) => Ok(()),
            Err(t_control::Error::NotFound) => a_control::interrupt(&id).map_err(a_error_control),
            Err(failure) => Err(error_control(failure)),
        }
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

fn merge_report(left: metrics::Report, right: metrics::Report) -> metrics::Report {
    let mut by_harness = left.by_harness;
    by_harness.extend(right.by_harness);
    metrics::Report {
        since_ms: left.since_ms.min(right.since_ms),
        until_ms: match (left.until_ms, right.until_ms) {
            (0, 0) => 0,
            (0, value) | (value, 0) => value,
            (left, right) => left.max(right),
        },
        agents: left.agents + right.agents,
        active: left.active + right.active,
        needs_you: left.needs_you + right.needs_you,
        input_tokens: left.input_tokens + right.input_tokens,
        output_tokens: left.output_tokens + right.output_tokens,
        turns: left.turns + right.turns,
        tool_calls: left.tool_calls + right.tool_calls,
        tool_errors: left.tool_errors + right.tool_errors,
        tokens_per_second_milli: left.tokens_per_second_milli + right.tokens_per_second_milli,
        measured_agents: left.measured_agents + right.measured_agents,
        by_harness,
    }
}

impl metrics::Guest for Component {
    fn stats(since_ms: Option<u64>, until_ms: Option<u64>) -> Result<metrics::Report, p::Error> {
        let report = match t_metrics::stats(since_ms, until_ms) {
            Ok(value) => report(value),
            Err(t_metrics::Error::NotReady | t_metrics::Error::Unsupported) => metrics::Report {
                since_ms: since_ms.unwrap_or(0),
                until_ms: until_ms.unwrap_or(0),
                agents: 0,
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
            },
            Err(failure) => return Err(error_metrics(failure)),
        };
        match a_metrics::stats(since_ms, until_ms) {
            Ok(other) => Ok(merge_report(report, a_report(other))),
            Err(_) => Ok(report),
        }
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
        if task.harness == "acp" {
            let started = a_dispatch::dispatch(&a_to_task(task)).map_err(a_error_dispatch)?;
            return Ok(dispatch::Started {
                agent_id: started.agent_id,
                conversation: started.conversation,
            });
        }
        let started = t_dispatch::dispatch(&to_task(task)).map_err(error_dispatch)?;
        Ok(dispatch::Started {
            agent_id: started.agent_id,
            conversation: started.conversation,
        })
    }
}

impl cli::Guest for Component {
    fn run(args: Vec<String>) -> Result<String, String> {
        match args.first().map(String::as_str) {
            Some("agents") | Some("sessions") | None => {
                let agents = <Component as observe::Guest>::agents()
                    .map_err(|error| format!("{error:?}"))?;
                let rows: Vec<serde_json::Value> = agents
                    .into_iter()
                    .map(|agent| {
                        serde_json::json!({
                            "id": agent.id,
                            "harness": agent.harness,
                            "title": agent.title,
                            "phase": format!("{:?}", agent.phase),
                        })
                    })
                    .collect();
                serde_json::to_string(&rows).map_err(|error| error.to_string())
            }
            _ => t_cli::run(&args),
        }
    }
}
