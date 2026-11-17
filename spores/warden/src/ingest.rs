//! The forge adapter: GitHub's GraphQL pages in, a snapshot out.
//!
//! This lives in Rust rather than in the shell that calls it because a `jq`
//! program that maps forty fields is a program nobody reviews and nobody tests.
//! The pipeline is then one line — `gh api graphql --paginate | warden ingest` —
//! and the mapping has a test per shape the forge actually returns.
//!
//! The input is a stream of JSON documents, not one document. `gh` emits one per
//! page, and collecting several owners is therefore several queries whose pages
//! concatenate. Every document is read until the stream ends.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::snapshot::{RepoState, SCHEMA, Snapshot};

/// A collector accumulates pages and hands back one snapshot.
#[derive(Debug, Default)]
pub struct Collector {
    repos: BTreeMap<String, RepoState>,
    orgs: Vec<String>,
}

impl Collector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read one GraphQL response, adding the repositories it names.
    ///
    /// A page that carries `errors` and no data is not an error here: a fleet
    /// collection is allowed to lose one owner and still produce a report, and
    /// the pages that did arrive are the evidence for the rest.
    pub fn absorb(&mut self, document: &Value) -> usize {
        let parsed: GraphQlDocument = match serde_json::from_value(document.clone()) {
            Ok(parsed) => parsed,
            Err(_) => return 0,
        };
        let Some(owner) = parsed.data.and_then(|data| data.repository_owner) else {
            return 0;
        };
        let Some(login) = owner.login.filter(|login| !login.is_empty()) else {
            return 0;
        };
        if !self.orgs.iter().any(|org| org == &login) {
            self.orgs.push(login.clone());
        }
        let Some(connection) = owner.repositories else {
            return 0;
        };

        let mut added = 0;
        for node in connection.nodes.into_iter().flatten() {
            let Some(name) = node.name.filter(|name| !name.is_empty()) else {
                continue;
            };
            self.repos.insert(
                format!("{login}/{name}"),
                RepoState {
                    org: login.clone(),
                    name,
                    archived: node.is_archived,
                    fork: node.is_fork,
                    private: node.is_private,
                    pushed_at: non_empty(node.pushed_at),
                    default_branch: node
                        .default_branch_ref
                        .and_then(|branch| non_empty(branch.name)),
                    license: node
                        .license_info
                        .and_then(|license| non_empty(license.spdx_id)),
                    description: node.description.filter(|text| !text.trim().is_empty()),
                    releases: node.releases.map_or(0, |releases| releases.total_count),
                    open_pulls: node.pull_requests.map_or(0, |pulls| pulls.total_count),
                    branches: node.branches.map_or(0, |branches| branches.total_count),
                    workflows: node
                        .workflows
                        .and_then(|tree| tree.entries)
                        .map(|entries| {
                            let mut names: Vec<String> = entries
                                .into_iter()
                                .flatten()
                                .filter_map(|entry| entry.name)
                                .collect();
                            names.sort();
                            names
                        })
                        .unwrap_or_default(),
                    latest_run: None,
                    links: None,
                    exempt: BTreeMap::new(),
                },
            );
            added += 1;
        }
        added
    }

    /// Read every document in a stream, returning how many repositories arrived.
    pub fn absorb_stream(&mut self, bytes: &[u8]) -> Result<usize, serde_json::Error> {
        let mut total = 0;
        let stream = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>();
        for document in stream {
            total += self.absorb(&document?);
        }
        Ok(total)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.repos.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    /// Owners seen, in the order their first page arrived.
    #[must_use]
    pub fn orgs(&self) -> &[String] {
        &self.orgs
    }

    #[must_use]
    pub fn finish(self, collected_at: Option<String>) -> Snapshot {
        let mut repos: Vec<RepoState> = self.repos.into_values().collect();
        repos.sort_by_key(|left| left.slug());
        Snapshot {
            schema: SCHEMA,
            collected_at,
            policy: crate::Policy::default(),
            repos,
        }
    }
}

fn non_empty(text: Option<String>) -> Option<String> {
    text.filter(|text| !text.trim().is_empty())
}

#[derive(Debug, Deserialize)]
struct GraphQlDocument {
    #[serde(default)]
    data: Option<GraphQlData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlData {
    #[serde(default)]
    repository_owner: Option<Owner>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Owner {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    repositories: Option<Connection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    #[serde(default)]
    nodes: Vec<Option<Node>>,
}

// Not `deny_unknown_fields`: the forge grows fields, and a collector that
// refuses to read a page because GitHub added one is a collector that stops
// working on somebody else's release day.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    is_archived: bool,
    #[serde(default)]
    is_fork: bool,
    #[serde(default)]
    is_private: bool,
    #[serde(default)]
    pushed_at: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license_info: Option<LicenseInfo>,
    #[serde(default)]
    default_branch_ref: Option<BranchRef>,
    #[serde(default)]
    releases: Option<Count>,
    #[serde(default)]
    pull_requests: Option<Count>,
    #[serde(default)]
    #[serde(rename = "branches")]
    branches: Option<Count>,
    #[serde(default)]
    workflows: Option<Tree>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LicenseInfo {
    #[serde(default)]
    spdx_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchRef {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    #[serde(default)]
    total_count: u32,
}

#[derive(Debug, Deserialize)]
struct Tree {
    #[serde(default)]
    entries: Option<Vec<Option<Entry>>>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    #[serde(default)]
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::Collector;

    const PAGE: &str = r#"
    {"data":{"repositoryOwner":{"login":"fpl","repositories":{"nodes":[
      {"name":"bob","isArchived":false,"isFork":false,"isPrivate":true,
       "pushedAt":"2026-11-17T10:54:14Z","description":"a build plane",
       "licenseInfo":null,"defaultBranchRef":{"name":"main"},
       "releases":{"totalCount":3},"pullRequests":{"totalCount":2},
       "branches":{"totalCount":28},
       "workflows":{"entries":[{"name":"release.yml"},{"name":"ci.yml"}]}},
      {"name":"fab","isArchived":false,"isFork":false,"isPrivate":true,
       "pushedAt":"2026-11-17T09:38:28Z","description":null,
       "licenseInfo":{"spdxId":"MIT"},"defaultBranchRef":{"name":"master"},
       "releases":{"totalCount":0},"pullRequests":{"totalCount":1},
       "branches":{"totalCount":71},"workflows":null}
    ]}}}}
    "#;

    #[test]
    fn maps_a_graphql_page_to_repository_state() {
        let mut collector = Collector::new();
        assert_eq!(collector.absorb_stream(PAGE.as_bytes()).unwrap(), 2);
        let snapshot = collector.finish(Some("2026-11-17T12:00:00Z".into()));

        assert_eq!(snapshot.repos.len(), 2);
        assert_eq!(snapshot.repos[0].slug(), "fpl/bob");
        assert_eq!(snapshot.repos[0].license, None);
        assert_eq!(snapshot.repos[0].workflows, vec!["ci.yml", "release.yml"]);
        assert_eq!(snapshot.repos[0].branches, 28);
        assert_eq!(snapshot.repos[0].default_branch.as_deref(), Some("main"));

        assert_eq!(snapshot.repos[1].license.as_deref(), Some("MIT"));
        assert!(snapshot.repos[1].workflows.is_empty());
        assert_eq!(snapshot.repos[1].description, None);
    }

    #[test]
    fn several_pages_and_several_owners_concatenate() {
        let page = r#"{"data":{"repositoryOwner":{"login":"other","repositories":{"nodes":[{"name":"thing"}]}}}}"#;
        let mut collector = Collector::new();
        collector.absorb_stream(PAGE.as_bytes()).unwrap();
        collector.absorb_stream(page.as_bytes()).unwrap();
        let snapshot = collector.finish(None);
        assert_eq!(snapshot.repos.len(), 3);
        assert_eq!(
            snapshot.orgs(),
            vec!["fpl".to_string(), "other".to_string()]
        );
    }

    #[test]
    fn a_page_with_only_errors_contributes_nothing_rather_than_failing() {
        let mut collector = Collector::new();
        let errors = r#"{"data":{"repositoryOwner":null},"errors":[{"message":"nope"}]}"#;
        assert_eq!(collector.absorb_stream(errors.as_bytes()).unwrap(), 0);
        assert!(collector.is_empty());
    }

    #[test]
    fn a_repository_seen_twice_is_stored_once() {
        let mut collector = Collector::new();
        collector.absorb_stream(PAGE.as_bytes()).unwrap();
        collector.absorb_stream(PAGE.as_bytes()).unwrap();
        assert_eq!(collector.len(), 2);
    }

    #[test]
    fn an_unreadable_document_is_an_error_not_a_silent_empty_fleet() {
        let mut collector = Collector::new();
        assert!(collector.absorb_stream(b"{not json}").is_err());
    }
}
