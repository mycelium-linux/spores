//! The projections the page renders, as pure functions.
//!
//! Nothing here knows about HTTP or the component model, so the burndown
//! arithmetic, the kanban grouping and the readiness rule are checked by
//! ordinary `cargo test` on the host rather than by looking at a page.
//!
//! The kanban and burndown portion began as `spores/board/src/model.rs` — the
//! GPUI board draws the same two things — with dependency edges, readiness and
//! project/plan rollups added here. Keeping the source of truth in the record
//! body means both boards disagree with the store, never with each other about
//! what the store says.
//!
//! # The data
//!
//! A Marble arrives as one `soil:records` entry: entity id `<type>/<json-key>`
//! and a body that is the record's JSON state. Issues are `issue/[...]`;
//! dependency edges are `dependency/[...]` whose body names `child` and
//! `parent`. Every other type is ignored by this view.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The statuses a Marble moves through, in the order a board shows them.
pub const STATUSES: [&str; 6] = [
    "open",
    "in_progress",
    "review",
    "blocked",
    "deferred",
    "closed",
];

/// A declared dependency edge: `child` waits on `parent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub child: String,
    pub parent: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub value: String,
}

/// One Marble, as much of it as a page needs.
///
/// Every field is defaulted: a body written by an older tracker build is still
/// drawable, and a missing column widens to empty rather than failing the load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub issue_type: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub actor_kind: Option<String>,
    /// Epoch seconds.
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub closed_at: Option<i64>,
    #[serde(default)]
    pub available_at: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub external_ref: Option<String>,
}

/// A whole book: the issues, the edges, and the two indexes the page needs.
#[derive(Debug, Clone, Default)]
pub struct Book {
    pub issues: Vec<Issue>,
    pub edges: Vec<Edge>,
    /// child → the issues it waits on.
    pub parents: BTreeMap<String, Vec<String>>,
    /// parent → the issues waiting on it.
    pub children: BTreeMap<String, Vec<String>>,
    /// The newest moment in the book. A Spore has no wall clock; this is the
    /// clock the page runs on, and what makes a read reproducible.
    pub now: i64,
}

/// The record types a view does not draw. Prefix tests, not schema whitelists,
/// so a tracker that adds a type cannot silently hide issues.
const ISSUE_PREFIX: &str = "issue/";
const DEPENDENCY_PREFIX: &str = "dependency/";

/// The first key component of an entity id, or the whole tail when the key is
/// not JSON. A card with a strange label is better than a card with no name.
fn id_from_key(id: &str) -> String {
    let tail = id.split_once('/').map(|(_, tail)| tail).unwrap_or(id);
    match serde_json::from_str::<Vec<String>>(tail) {
        Ok(key) => key.into_iter().next().unwrap_or_else(|| tail.to_owned()),
        Err(_) => tail.to_owned(),
    }
}

/// Decode one live scan of a records collection into a [`Book`].
///
/// Deleted entries and bodies this build cannot read are skipped: a view that
/// refuses to draw because one row is odd is worse than a view that draws the
/// rows it understands.
#[must_use]
pub fn book(entries: &[(String, Vec<u8>)]) -> Book {
    let mut issues = Vec::new();
    let mut edges = Vec::new();
    for (id, body) in entries {
        if id.starts_with(ISSUE_PREFIX) {
            let Ok(mut issue) = serde_json::from_slice::<Issue>(body) else {
                continue;
            };
            if issue.id.is_empty() {
                issue.id = id_from_key(id);
            }
            issues.push(issue);
        } else if id.starts_with(DEPENDENCY_PREFIX) {
            if let Ok(edge) = serde_json::from_slice::<Edge>(body) {
                if !edge.child.is_empty() && !edge.parent.is_empty() {
                    edges.push(edge);
                }
            }
        }
    }
    issues.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut parents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in &edges {
        parents
            .entry(edge.child.clone())
            .or_default()
            .push(edge.parent.clone());
        children
            .entry(edge.parent.clone())
            .or_default()
            .push(edge.child.clone());
    }
    for values in parents.values_mut().chain(children.values_mut()) {
        values.sort();
        values.dedup();
    }
    let now = latest(&issues);
    Book {
        issues,
        edges,
        parents,
        children,
        now,
    }
}

/// The newest moment the book records.
#[must_use]
pub fn latest(issues: &[Issue]) -> i64 {
    issues
        .iter()
        .flat_map(|issue| {
            [
                issue.updated_at,
                issue.created_at,
                issue.closed_at.unwrap_or(0),
            ]
        })
        .max()
        .unwrap_or(0)
}

/// The heading a column wears. `open` is "Ready" because that is the tracker's
/// word for work nobody has claimed.
#[must_use]
pub fn column_title(status: &str) -> &'static str {
    match status {
        "open" => "Ready",
        "in_progress" => "In progress",
        "review" => "Review",
        "blocked" => "Blocked",
        "deferred" => "Deferred",
        "closed" => "Closed",
        _ => "Other",
    }
}

#[must_use]
pub fn is_terminal(status: &str) -> bool {
    status == "closed"
}

/// Whether one issue answers a search.
#[must_use]
pub fn matches(issue: &Issue, query: &str, project: Option<&str>) -> bool {
    if project.is_some_and(|project| issue.project != project) {
        return false;
    }
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    issue.id.to_lowercase().contains(&needle)
        || issue.title.to_lowercase().contains(&needle)
        || issue.project.to_lowercase().contains(&needle)
        || issue
            .labels
            .iter()
            .any(|label| label.to_lowercase().contains(&needle))
        || issue
            .assignee
            .as_deref()
            .is_some_and(|who| who.to_lowercase().contains(&needle))
}

/// Why an issue is not claimable right now. An empty answer means ready.
///
/// The rules are the tracker's: open, past its creation grace, not under a live
/// claim, and every local parent closed. A parent that is missing from the book
/// is named rather than ignored, because a silently dropped blocker turns
/// blocked work into a claim someone immediately has to release.
impl Book {
    /// An index by issue id, built once per page rather than once per issue.
    #[must_use]
    pub fn index(&self) -> BTreeMap<&str, &Issue> {
        self.issues
            .iter()
            .map(|issue| (issue.id.as_str(), issue))
            .collect()
    }
}

#[must_use]
pub fn blocking(book: &Book, issue: &Issue, now: i64) -> Vec<String> {
    blocking_in(book, &book.index(), issue, now)
}

/// [`blocking`] with the id index supplied, for a caller that projects many
/// issues and must not rebuild it per row.
#[must_use]
pub fn blocking_in(
    book: &Book,
    by_id: &BTreeMap<&str, &Issue>,
    issue: &Issue,
    now: i64,
) -> Vec<String> {
    if issue.status != "open" {
        return vec![format!("{} not open", issue.status)];
    }
    let mut reasons = Vec::new();
    if issue.available_at.is_some_and(|at| at > now) {
        reasons.push("inside its creation grace".to_string());
    }
    if let Some(who) = &issue.assignee {
        match issue.expires_at {
            Some(expires) if expires > now => reasons.push(format!("claimed by {who}")),
            _ => {}
        }
    }
    for parent in book
        .parents
        .get(&issue.id)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        match by_id.get(parent.as_str()) {
            Some(parent_issue) if is_terminal(&parent_issue.status) => {}
            Some(parent_issue) => {
                reasons.push(format!("waits on {parent} ({})", parent_issue.status))
            }
            None => reasons.push(format!("waits on missing {parent}")),
        }
    }
    reasons
}

#[must_use]
pub fn is_ready(book: &Book, issue: &Issue, now: i64) -> bool {
    blocking(book, issue, now).is_empty()
}

#[must_use]
pub fn is_ready_in(book: &Book, by_id: &BTreeMap<&str, &Issue>, issue: &Issue, now: i64) -> bool {
    blocking_in(book, by_id, issue, now).is_empty()
}

/// A card: the bounded projection a column draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub project: String,
    pub priority: i64,
    pub issue_type: String,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub status: String,
    pub title: &'static str,
    pub cards: Vec<Card>,
}

impl Column {
    #[must_use]
    pub fn count(&self) -> usize {
        self.cards.len()
    }
}

fn card(issue: &Issue) -> Card {
    Card {
        id: issue.id.clone(),
        title: issue.title.clone(),
        project: issue.project.clone(),
        priority: issue.priority,
        issue_type: issue.issue_type.clone(),
        assignee: issue.assignee.clone(),
        labels: issue.labels.clone(),
    }
}

/// Group the matching issues into the six status columns. An unknown status
/// still gets a column after the six rather than being dropped.
#[must_use]
pub fn columns(issues: &[Issue], query: &str, project: Option<&str>) -> Vec<Column> {
    let mut columns: Vec<Column> = STATUSES
        .iter()
        .map(|status| Column {
            status: (*status).to_owned(),
            title: column_title(status),
            cards: Vec::new(),
        })
        .collect();
    for issue in issues.iter().filter(|issue| matches(issue, query, project)) {
        let at = match columns
            .iter()
            .position(|column| column.status == issue.status)
        {
            Some(at) => at,
            None => {
                columns.push(Column {
                    title: column_title(&issue.status),
                    status: issue.status.clone(),
                    cards: Vec::new(),
                });
                columns.len() - 1
            }
        };
        columns[at].cards.push(card(issue));
    }
    columns
}

/// One day of the burndown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    /// Epoch seconds at the end of this day's bucket.
    pub at: i64,
    pub scope: i64,
    pub done: i64,
    pub remaining: i64,
    pub ideal: i64,
}

#[derive(Debug, Clone)]
pub struct Burndown {
    pub days: Vec<Day>,
    pub scope: i64,
    pub done: i64,
    pub remaining: i64,
    pub burn_per_day: Option<f64>,
    pub projected_days_to_zero: Option<f64>,
}

impl Burndown {
    #[must_use]
    pub fn peak(&self) -> i64 {
        self.days
            .iter()
            .map(|day| day.scope.max(day.remaining).max(day.ideal))
            .max()
            .unwrap_or(0)
            .max(1)
    }
}

/// Daily remaining-work series over the last `days` days, over the selected
/// subset. The ideal line burns the whole scope evenly across the window.
#[must_use]
pub fn burndown(issues: &[Issue], days: i64, now: i64) -> Burndown {
    let days = days.max(1);
    let span = days.saturating_mul(86_400);
    let start = now.saturating_sub(span);
    let total_scope = issues.len() as i64;
    let mut points = Vec::with_capacity(days as usize + 1);
    for d in 0..=days {
        let at = (start + d.saturating_mul(86_400)).min(now);
        let scope = issues.iter().filter(|i| i.created_at <= at).count() as i64;
        let done = issues
            .iter()
            .filter(|i| i.created_at <= at && i.closed_at.is_some_and(|c| c <= at))
            .count() as i64;
        let ideal = ((total_scope as f64) * (1.0 - (d as f64 / days as f64))).round() as i64;
        points.push(Day {
            at,
            scope,
            done,
            remaining: (scope - done).max(0),
            ideal: ideal.max(0),
        });
    }
    let done_at_start = issues
        .iter()
        .filter(|i| i.closed_at.is_some_and(|c| c <= start))
        .count() as i64;
    let done_now = issues
        .iter()
        .filter(|i| i.closed_at.is_some_and(|c| c <= now))
        .count() as i64;
    let elapsed_days = (now - start) as f64 / 86_400.0;
    let burn_per_day = if elapsed_days > 0.0 && done_now > done_at_start {
        Some((done_now - done_at_start) as f64 / elapsed_days)
    } else {
        None
    };
    let remaining = (total_scope - done_now).max(0);
    Burndown {
        days: points,
        scope: total_scope,
        done: done_now,
        remaining,
        burn_per_day,
        projected_days_to_zero: burn_per_day
            .filter(|rate| *rate > 0.0)
            .map(|rate| remaining as f64 / rate),
    }
}

/// A civil date, from days since the Unix epoch.
///
/// Howard Hinnant's `civil_from_days`, so the guest needs no calendar
/// dependency and no clock.
#[must_use]
pub fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[must_use]
pub fn date_label(at: i64) -> String {
    let (y, m, d) = civil_date(at.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// The headline counts on the dashboard.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub total: i64,
    pub open: i64,
    pub in_progress: i64,
    pub review: i64,
    pub blocked: i64,
    pub deferred: i64,
    pub closed: i64,
    pub goals: i64,
    pub epics: i64,
    pub tasks: i64,
    pub ready: i64,
    pub stale: i64,
    pub percent_done: f64,
}

impl Summary {
    #[must_use]
    pub fn from(book: &Book, project: Option<&str>, stale_days: i64, now: i64) -> Self {
        let stale_before = now.saturating_sub(stale_days.max(1).saturating_mul(86_400));
        let by_id = book.index();
        let mut out = Summary::default();
        for issue in book
            .issues
            .iter()
            .filter(|issue| project.is_none_or(|project| issue.project == project))
        {
            out.total += 1;
            match issue.status.as_str() {
                "open" => out.open += 1,
                "in_progress" => out.in_progress += 1,
                "review" => out.review += 1,
                "blocked" => out.blocked += 1,
                "deferred" => out.deferred += 1,
                "closed" => out.closed += 1,
                _ => {}
            }
            match issue.issue_type.as_str() {
                "goal" => out.goals += 1,
                "epic" => out.epics += 1,
                "task" => out.tasks += 1,
                _ => {}
            }
            if is_ready_in(book, &by_id, issue, now) {
                out.ready += 1;
            }
            if matches!(issue.status.as_str(), "open" | "in_progress")
                && issue.updated_at < stale_before
            {
                out.stale += 1;
            }
        }
        out.percent_done = if out.total > 0 {
            (out.closed as f64 / out.total as f64 * 1000.0).round() / 10.0
        } else {
            0.0
        };
        out
    }
}

/// One row of the project rollup.
#[derive(Debug, Clone, Default)]
pub struct ProjectStat {
    pub slug: String,
    pub total: i64,
    pub open: i64,
    pub in_progress: i64,
    pub review: i64,
    pub blocked: i64,
    pub closed: i64,
    pub ready: i64,
    pub stale: i64,
    pub newest: i64,
}

#[must_use]
pub fn projects(book: &Book, stale_days: i64, now: i64) -> Vec<ProjectStat> {
    let stale_before = now.saturating_sub(stale_days.max(1).saturating_mul(86_400));
    let by_id = book.index();
    let mut by_project: BTreeMap<String, ProjectStat> = BTreeMap::new();
    for issue in &book.issues {
        let entry = by_project
            .entry(issue.project.clone())
            .or_insert_with(|| ProjectStat {
                slug: issue.project.clone(),
                ..ProjectStat::default()
            });
        entry.total += 1;
        entry.newest = entry.newest.max(issue.updated_at);
        match issue.status.as_str() {
            "open" => entry.open += 1,
            "in_progress" => entry.in_progress += 1,
            "review" => entry.review += 1,
            "blocked" => entry.blocked += 1,
            "closed" => entry.closed += 1,
            _ => {}
        }
        if is_ready_in(book, &by_id, issue, now) {
            entry.ready += 1;
        }
        if matches!(issue.status.as_str(), "open" | "in_progress")
            && issue.updated_at < stale_before
        {
            entry.stale += 1;
        }
    }
    let mut rows: Vec<ProjectStat> = by_project.into_values().collect();
    rows.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.slug.cmp(&b.slug)));
    rows
}

/// One plan: issues whose metadata carries `plan = <name>`.
#[derive(Debug, Clone, Default)]
pub struct PlanStat {
    pub name: String,
    pub nodes: i64,
    pub closed: i64,
    pub ready: i64,
    pub blocked: i64,
}

#[must_use]
pub fn plan_name(issue: &Issue) -> Option<&str> {
    issue
        .metadata
        .get("plan")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
}

#[must_use]
pub fn plans(book: &Book, now: i64) -> Vec<PlanStat> {
    let by_id = book.index();
    let mut by_name: BTreeMap<String, PlanStat> = BTreeMap::new();
    for issue in &book.issues {
        let Some(name) = plan_name(issue) else {
            continue;
        };
        let entry = by_name.entry(name.to_string()).or_insert_with(|| PlanStat {
            name: name.to_string(),
            ..PlanStat::default()
        });
        entry.nodes += 1;
        if issue.status == "closed" {
            entry.closed += 1;
        } else if blocking_in(book, &by_id, issue, now).is_empty() {
            entry.ready += 1;
        } else {
            entry.blocked += 1;
        }
    }
    let mut rows: Vec<PlanStat> = by_name.into_values().collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// The nodes of one plan, in the tracker's own order.
#[must_use]
pub fn plan_nodes<'a>(book: &'a Book, name: &str) -> Vec<&'a Issue> {
    let mut nodes: Vec<&Issue> = book
        .issues
        .iter()
        .filter(|issue| plan_name(issue) == Some(name))
        .collect();
    nodes.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    nodes
}

/// The distinct projects in the book, for the filter control.
#[must_use]
pub fn project_slugs(book: &Book) -> Vec<String> {
    let mut slugs: BTreeSet<String> = book
        .issues
        .iter()
        .map(|issue| issue.project.clone())
        .filter(|slug| !slug.is_empty())
        .collect();
    std::mem::take(&mut slugs).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(id: &str, status: &str, created: i64, closed: Option<i64>) -> Issue {
        Issue {
            id: id.to_owned(),
            title: format!("work {id}"),
            project: "p".into(),
            status: status.to_owned(),
            issue_type: "task".into(),
            created_at: created,
            updated_at: created,
            closed_at: closed,
            ..Issue::default()
        }
    }

    fn entry(id: &str, body: &str) -> (String, Vec<u8>) {
        (id.to_string(), body.as_bytes().to_vec())
    }

    #[test]
    fn other_record_types_are_not_issues() {
        let entries = vec![
            entry("issue/[\"a\"]", r#"{"id":"a","title":"A"}"#),
            entry("project/[\"p\"]", r#"{"slug":"p"}"#),
            entry("marbles_event/[\"1\"]", r#"{}"#),
        ];
        assert_eq!(book(&entries).issues.len(), 1);
    }

    #[test]
    fn a_body_without_an_id_takes_the_one_from_the_key() {
        let entries = vec![entry("issue/[\"mb-7\"]", r#"{"title":"T"}"#)];
        assert_eq!(book(&entries).issues[0].id, "mb-7");
    }

    #[test]
    fn edges_are_indexed_both_ways_and_block_readiness() {
        let entries = vec![
            entry("issue/[\"a\"]", r#"{"id":"a","status":"open"}"#),
            entry("issue/[\"b\"]", r#"{"id":"b","status":"open"}"#),
            entry(
                "dependency/[\"b\\u001fa\"]",
                r#"{"child":"b","parent":"a"}"#,
            ),
        ];
        let book = book(&entries);
        let b = book.issues.iter().find(|issue| issue.id == "b").unwrap();
        let a = book.issues.iter().find(|issue| issue.id == "a").unwrap();
        assert_eq!(book.parents["b"], vec!["a"]);
        assert_eq!(book.children["a"], vec!["b"]);
        assert!(is_ready(&book, a, 10));
        assert!(!is_ready(&book, b, 10));
        assert!(blocking(&book, b, 10)[0].contains("waits on a"));
    }

    #[test]
    fn a_grace_window_and_a_live_claim_block_but_an_expired_one_does_not() {
        let mut scheduled = issue("a", "open", 0, None);
        scheduled.available_at = Some(100);
        let mut claimed = issue("b", "open", 0, None);
        claimed.assignee = Some("ada".into());
        claimed.expires_at = Some(50);
        let book = book(&[
            entry("issue/[\"a\"]", &serde_json::to_string(&scheduled).unwrap()),
            entry("issue/[\"b\"]", &serde_json::to_string(&claimed).unwrap()),
        ]);
        let a = book.issues.iter().find(|issue| issue.id == "a").unwrap();
        let b = book.issues.iter().find(|issue| issue.id == "b").unwrap();
        assert!(!is_ready(&book, a, 10));
        assert!(is_ready(&book, a, 200));
        assert!(!is_ready(&book, b, 10));
        assert!(!is_ready(&book, b, 20));
        assert!(is_ready(&book, b, 60));
    }

    #[test]
    fn search_covers_the_fields_a_person_types() {
        let mut a = issue("mb-1", "open", 0, None);
        a.title = "Fix the parser".into();
        a.labels = vec!["bug".into()];
        a.assignee = Some("ada".into());
        assert!(matches(&a, "mb-1", None));
        assert!(matches(&a, "PARSER", None));
        assert!(matches(&a, "bug", None));
        assert!(matches(&a, "ada", None));
        assert!(!matches(&a, "zebra", None));
        assert!(!matches(&a, "mb-1", Some("other")));
    }

    #[test]
    fn columns_keep_unknown_statuses_after_the_six() {
        let issues = vec![
            issue("a", "open", 0, None),
            issue("b", "closed", 0, Some(1)),
            issue("c", "waiting", 0, None),
        ];
        let columns = columns(&issues, "", None);
        assert_eq!(columns.len(), 7);
        assert_eq!(columns[0].status, "open");
        assert!(columns.iter().any(|column| column.status == "waiting"));
    }

    #[test]
    fn burndown_burns_a_three_day_window() {
        let day = 86_400;
        let now = 100 * day;
        let issues = vec![
            issue("a", "closed", 98 * day, Some(99 * day)),
            issue("b", "open", 98 * day, None),
        ];
        let burn = burndown(&issues, 3, now);
        assert_eq!(burn.scope, 2);
        assert_eq!(burn.days.len(), 4);
        let last = burn.days.last().unwrap();
        assert_eq!((last.scope, last.done, last.remaining), (2, 1, 1));
        assert!(burn.burn_per_day.is_some());
    }

    #[test]
    fn dates_are_civil() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(date_label(0), "1970-01-01");
    }

    #[test]
    fn plans_group_by_metadata_and_ready_counts_ignore_container_types() {
        let mut node = issue("n", "open", 0, None);
        node.metadata = serde_json::json!({"plan": "ship"});
        let mut other = issue("o", "closed", 0, Some(1));
        other.metadata = serde_json::json!({"plan": "ship"});
        let book = book(&[
            entry("issue/[\"n\"]", &serde_json::to_string(&node).unwrap()),
            entry("issue/[\"o\"]", &serde_json::to_string(&other).unwrap()),
            entry("issue/[\"x\"]", r#"{"id":"x","status":"open"}"#),
        ]);
        let plans = plans(&book, 10);
        assert_eq!(plans.len(), 1);
        assert_eq!((plans[0].nodes, plans[0].closed, plans[0].ready), (2, 1, 1));
        assert_eq!(plan_nodes(&book, "ship").len(), 2);
    }
}
