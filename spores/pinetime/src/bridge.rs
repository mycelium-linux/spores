//! The JARVIS decision contract as the watch sees it.
//!
//! This module is deliberately transport-free: it parses bus bodies, maps a
//! watch button to an answer, and builds the delivery/resolution bodies. The
//! Spore above it owns the BLE link and the poll loop.
//!
//! Sources:
//! - `docs/watch-bridge.md` in the Spores repository
//! - `jarvis-vpwj` in the Marbles tracker

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A decision published as `jarvis.decision.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub person: Option<String>,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
}

/// A notice published as `jarvis.notice.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub person: Option<String>,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
}

/// A `jarvis.reply.v1` frame. The watch only shows a final reply addressed to
/// it; partials and replies for other surfaces stay on the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub id: String,
    pub text: String,
    pub state: String,
    pub surface: Option<String>,
    pub person: Option<String>,
    pub at_ms: u64,
}

/// A decision another surface already answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub decision_id: String,
    pub surface: String,
    pub at_ms: u64,
}

/// The three buttons InfiniTime exposes on a call notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchResponse {
    Reject,
    Accept,
    Mute,
}

impl WatchResponse {
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::Reject),
            0x01 => Some(Self::Accept),
            0x02 => Some(Self::Mute),
            _ => None,
        }
    }
}

/// Parse a `jarvis.decision.v1` body. `summary` is preferred over `question`
/// because the domain calls the short form `summary`; older publishers used
/// `question` before the contract settled.
#[must_use]
pub fn parse_decision(value: &Value) -> Option<Decision> {
    let id = string(value, "decision_id").or_else(|| string(value, "id"))?;
    let summary = string(value, "summary").or_else(|| string(value, "question"))?;
    let detail = string(value, "consequence")
        .or_else(|| string(value, "detail"))
        .unwrap_or_default();
    let options = value
        .get("options")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(option_id).collect())
        .unwrap_or_default();
    Some(Decision {
        id,
        summary,
        detail,
        options,
        person: string(value, "person"),
        created_at_ms: u64(value, "created_at_ms"),
        expires_at_ms: value.get("expires_at_ms").and_then(Value::as_u64),
    })
}

/// Parse a `jarvis.notice.v1` body.
#[must_use]
pub fn parse_notice(value: &Value) -> Option<Notice> {
    let id = string(value, "notice_id").or_else(|| string(value, "id"))?;
    let title = string(value, "title")?;
    Some(Notice {
        id,
        title,
        body: string(value, "body").unwrap_or_default(),
        person: string(value, "person"),
        created_at_ms: u64(value, "created_at_ms"),
        expires_at_ms: value.get("expires_at_ms").and_then(Value::as_u64),
    })
}

/// Parse a `jarvis.reply.v1` body.
#[must_use]
pub fn parse_reply(value: &Value) -> Option<Reply> {
    Some(Reply {
        id: string(value, "reply_id").or_else(|| string(value, "id"))?,
        text: string(value, "text")?,
        state: string(value, "state").unwrap_or_else(|| "final".to_string()),
        surface: value
            .get("delivery")
            .and_then(|delivery| delivery.get("surface"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        person: string(value, "person"),
        at_ms: u64(value, "at_ms"),
    })
}

/// Parse a `jarvis.decision.resolve.v1` body.
#[must_use]
pub fn parse_resolution(value: &Value) -> Option<Resolution> {
    Some(Resolution {
        decision_id: string(value, "decision_id")?,
        surface: value
            .get("by")
            .and_then(|by| by.get("surface"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        at_ms: u64(value, "at_ms"),
    })
}

/// Whether a decision is honestly a yes/no approval.
///
/// The wrist only has two meaningful buttons today, so a binary choice with
/// arbitrary labels (`ship` / `hold`) is not a thumbs question. An adapter that
/// wants thumbs should normalize an approval prompt to a canonical option set.
#[must_use]
pub fn is_approval(options: &[String]) -> bool {
    if options.is_empty() {
        return true;
    }
    if options.len() > 2 {
        return false;
    }
    let yes = options.iter().any(|option| {
        ["yes", "approve", "accept", "allow", "ok"]
            .iter()
            .any(|name| option.eq_ignore_ascii_case(name))
    });
    let no = options.iter().any(|option| {
        ["no", "deny", "reject", "decline", "block", "cancel"]
            .iter()
            .any(|name| option.eq_ignore_ascii_case(name))
    });
    yes && no
}

/// The answer to publish for a watch button.
///
/// A decision may offer `yes`/`no`, an arbitrary option list, or no options at
/// all. Accept chooses the affirmative option (or the first), reject chooses the
/// negative option (or the last), and mute is not an answer at all: it silences
/// this surface and leaves the decision open for someone else.
#[must_use]
pub fn answer_for(response: WatchResponse, options: &[String]) -> Option<String> {
    match response {
        WatchResponse::Mute => None,
        WatchResponse::Accept => Some(
            find_option(options, &["yes", "accept", "approve"])
                .or_else(|| options.first().cloned())
                .unwrap_or_else(|| "yes".to_string()),
        ),
        WatchResponse::Reject => Some(
            find_option(options, &["no", "reject", "decline"])
                .or_else(|| options.last().cloned())
                .unwrap_or_else(|| "no".to_string()),
        ),
    }
}

/// `jarvis.delivery.v1`: what a surface did with an item.
#[must_use]
pub fn delivery(item_id: &str, state: &str, at_ms: u64) -> Value {
    json!({
        "item_id": item_id,
        "surface": "watch",
        "state": state,
        "at_ms": at_ms,
    })
}

/// `jarvis.decision.resolve.v1`: the watch's answer.
#[must_use]
pub fn resolution(decision_id: &str, answer: &str, actor: &str, at_ms: u64) -> Value {
    json!({
        "decision_id": decision_id,
        "answer": answer,
        "by": { "surface": "watch", "actor": actor },
        "at_ms": at_ms,
    })
}

fn find_option(options: &[String], names: &[&str]) -> Option<String> {
    options
        .iter()
        .find(|option| names.iter().any(|name| option.eq_ignore_ascii_case(name)))
        .cloned()
}

fn option_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.get("id").and_then(Value::as_str).map(str::to_owned))
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_decision_with_string_options() {
        let value = json!({
            "decision_id": "d-1",
            "summary": "Deploy?",
            "consequence": "Two minutes down",
            "options": ["yes", "no"],
            "person": "avery",
            "created_at_ms": 10,
            "expires_at_ms": 20
        });
        let decision = parse_decision(&value).expect("decision");
        assert_eq!(decision.id, "d-1");
        assert_eq!(decision.summary, "Deploy?");
        assert_eq!(decision.detail, "Two minutes down");
        assert_eq!(decision.options, vec!["yes", "no"]);
        assert_eq!(decision.person.as_deref(), Some("avery"));
        assert_eq!(decision.expires_at_ms, Some(20));
    }

    #[test]
    fn parses_choice_objects_and_the_question_fallback() {
        let value = json!({
            "id": "d-2",
            "question": "Which one?",
            "options": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]
        });
        let decision = parse_decision(&value).expect("decision");
        assert_eq!(decision.summary, "Which one?");
        assert_eq!(decision.options, vec!["a", "b"]);
    }

    #[test]
    fn approval_is_a_yes_no_question_not_any_binary() {
        assert!(is_approval(&[]));
        assert!(is_approval(&["yes".into(), "no".into()]));
        assert!(is_approval(&["approve".into(), "deny".into()]));
        assert!(!is_approval(&["ship".into(), "hold".into()]));
        assert!(!is_approval(&["yes".into(), "no".into(), "later".into()]));
    }

    #[test]
    fn every_approval_set_is_answerable_by_both_buttons() {
        for options in [
            vec![],
            vec!["yes".to_string(), "no".to_string()],
            vec!["approve".to_string(), "deny".to_string()],
            vec!["OK".to_string(), "Cancel".to_string()],
        ] {
            assert!(is_approval(&options), "{options:?}");
            assert!(answer_for(WatchResponse::Accept, &options).is_some());
            assert!(answer_for(WatchResponse::Reject, &options).is_some());
        }
    }

    #[test]
    fn accept_and_reject_pick_the_domain_options() {
        let options = vec!["yes".to_string(), "no".to_string()];
        assert_eq!(
            answer_for(WatchResponse::Accept, &options).as_deref(),
            Some("yes")
        );
        assert_eq!(
            answer_for(WatchResponse::Reject, &options).as_deref(),
            Some("no")
        );
        assert_eq!(answer_for(WatchResponse::Mute, &options), None);
    }

    #[test]
    fn arbitrary_options_use_the_first_and_last() {
        let options = vec!["ship".to_string(), "hold".to_string()];
        assert_eq!(
            answer_for(WatchResponse::Accept, &options).as_deref(),
            Some("ship")
        );
        assert_eq!(
            answer_for(WatchResponse::Reject, &options).as_deref(),
            Some("hold")
        );
    }

    #[test]
    fn resolution_body_names_the_watch_as_the_surface() {
        let body = resolution("d-1", "yes", "avery", 42);
        assert_eq!(body["decision_id"], "d-1");
        assert_eq!(body["answer"], "yes");
        assert_eq!(body["by"]["surface"], "watch");
        assert_eq!(body["by"]["actor"], "avery");
    }

    #[test]
    fn parses_a_final_reply_for_the_watch() {
        let value = json!({
            "reply_id": "r-1",
            "query_id": "q-1",
            "text": "Deploying now",
            "state": "final",
            "delivery": {"surface": "watch"},
            "person": "avery",
            "at_ms": 42
        });
        let reply = parse_reply(&value).expect("reply");
        assert_eq!(reply.id, "r-1");
        assert_eq!(reply.text, "Deploying now");
        assert_eq!(reply.state, "final");
        assert_eq!(reply.surface.as_deref(), Some("watch"));
        assert_eq!(reply.person.as_deref(), Some("avery"));
        assert_eq!(reply.at_ms, 42);
    }

    #[test]
    fn parses_another_surfaces_resolution() {
        let value = json!({
            "decision_id": "d-1",
            "answer": "yes",
            "by": {"surface": "voice", "actor": "avery"},
            "at_ms": 42
        });
        let resolution = parse_resolution(&value).expect("resolution");
        assert_eq!(resolution.decision_id, "d-1");
        assert_eq!(resolution.surface, "voice");
        assert_eq!(resolution.at_ms, 42);
    }
}
