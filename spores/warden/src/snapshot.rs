//! The snapshot: everything warden knows about a repository, as one document.
//!
//! A snapshot is a *fact sheet*, not a judgement. It records what the forge
//! said at a moment; every opinion about it lives in [`crate::policy`]. That
//! split is what lets the same bytes be re-evaluated next month under a
//! different policy without collecting again.
//!
//! The produced shape is deliberately boring JSON so a collector can be a shell
//! script, a GitHub Action, a Spore, or a person with a text editor.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The snapshot schema this build reads and writes.
pub const SCHEMA: u32 = 1;

fn schema_version() -> u32 {
    SCHEMA
}

/// One repository, as the forge described it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoState {
    /// The owner login, exactly as the forge spells it.
    pub org: String,
    /// The repository name, without the owner.
    pub name: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub fork: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,
    /// RFC 3339, the time of the last push to any branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// The SPDX id the forge detected, which is `None` when it found no licence
    /// file it could name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Release count, not the releases themselves: the question is whether the
    /// project ever shipped, and the list is megabytes for a fleet this size.
    #[serde(default)]
    pub releases: u32,
    #[serde(default)]
    pub open_pulls: u32,
    #[serde(default)]
    pub branches: u32,
    /// Workflow file names under `.github/workflows`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<String>,
    /// Evidence the first collector could not afford, and a fuller one can.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_run: Option<Run>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<Link>>,
    /// Checks this repository does not need, and why. Reviewed like a grant:
    /// intent is written down, so `license: "proprietary, see legal"` is a
    /// decision instead of a hole the report cannot see.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exempt: BTreeMap<String, String>,
}

impl RepoState {
    /// `owner/name`, the form every message uses.
    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.org, self.name)
    }
}

/// The last workflow run on the default branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow: String,
    /// The forge's conclusion: `success`, `failure`, `cancelled`, ... An empty
    /// string is a run that has not concluded, which is `unknown` and not green.
    #[serde(default)]
    pub conclusion: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// One documentation link and what happened when it was fetched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub url: String,
    /// The HTTP status, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The transport error, when there was one instead of a status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Everything one collection pass produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    #[serde(default = "schema_version")]
    pub schema: u32,
    /// When the collector looked. `None` is honest: a hand-written snapshot has
    /// no clock behind it, and the checks that need one answer `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collected_at: Option<String>,
    /// The policy this snapshot is evaluated under. Travels with the data so a
    /// report can be reproduced from the bytes alone.
    #[serde(default)]
    pub policy: crate::Policy,
    pub repos: Vec<RepoState>,
}

impl Snapshot {
    /// The owners this snapshot covers, in the order they first appear.
    #[must_use]
    pub fn orgs(&self) -> Vec<String> {
        let mut orgs: Vec<String> = Vec::new();
        for repo in &self.repos {
            if !orgs.iter().any(|org| org == &repo.org) {
                orgs.push(repo.org.clone());
            }
        }
        orgs
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::{RepoState, Snapshot};

    #[test]
    fn a_minimal_repository_needs_only_an_identity() {
        let state: RepoState = serde_json::from_str(r#"{"org":"fpl","name":"bob"}"#).unwrap();
        assert_eq!(state.slug(), "fpl/bob");
        assert!(!state.archived);
        assert!(state.workflows.is_empty());
    }

    #[test]
    fn defaults_fill_in_for_a_hand_written_snapshot() {
        let snapshot: Snapshot =
            serde_json::from_str(r#"{"repos":[{"org":"a","name":"b"}]}"#).unwrap();
        assert_eq!(snapshot.schema, super::SCHEMA);
        assert_eq!(snapshot.collected_at, None);
        assert_eq!(snapshot.policy, crate::Policy::default());
    }

    #[test]
    fn unknown_fields_are_refused_rather_than_dropped() {
        // A collector that invents a field is a collector whose evidence the
        // report will silently ignore. Refusing is the honest answer.
        let error = serde_json::from_str::<RepoState>(r#"{"org":"a","name":"b","stars":3}"#);
        assert!(error.is_err());
    }

    #[test]
    fn orgs_come_out_in_first_seen_order_without_duplicates() {
        let snapshot = Snapshot {
            schema: super::SCHEMA,
            collected_at: None,
            policy: crate::Policy::default(),
            repos: vec![
                RepoState {
                    org: "z".into(),
                    name: "one".into(),
                    ..RepoState::default()
                },
                RepoState {
                    org: "a".into(),
                    name: "two".into(),
                    ..RepoState::default()
                },
                RepoState {
                    org: "z".into(),
                    name: "three".into(),
                    ..RepoState::default()
                },
            ],
        };
        assert_eq!(snapshot.orgs(), vec!["z".to_string(), "a".to_string()]);
    }
}
