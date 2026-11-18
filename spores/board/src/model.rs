//! The three projections the board draws, as pure functions.
//!
//! Nothing here knows about GPUI, WebGPU or the component model. That is
//! deliberate: the burndown arithmetic, the column grouping and the search are
//! what a reviewer needs to be able to check, and they are checked by ordinary
//! `cargo test` on the host rather than by looking at a screenshot.
//!
//! # The data
//!
//! A Marble arrives as one `soil:records` entry: entity id `<type>/<json-key>`
//! and a body that is the issue's JSON state. The board reads the `issue`
//! records and ignores every other type, so a narrower or wider records book
//! changes what there is to draw, never what the drawing means.

use serde::Deserialize;

/// The statuses a Marble moves through, in the order a board shows them.
///
/// This mirrors `marbles`' own vocabulary rather than inventing a second one:
/// the record body carries these words, and a column that renamed them would be
/// a view disagreeing with the store.
pub const STATUSES: [&str; 6] = [
    "open",
    "in_progress",
    "review",
    "blocked",
    "deferred",
    "closed",
];

/// The heading a column wears. `open` is "Ready" because that is the word the
/// tracker uses for work nobody has claimed.
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

/// One Marble, as much of it as a board needs.
///
/// Deserialized from the issue body the records book holds. Every field is
/// defaulted: a body written by an older tracker build is still drawable, and a
/// missing column should widen the card to empty rather than fail the load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
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
    /// Epoch seconds.
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub closed_at: Option<i64>,
}

/// The record types a board does not draw. Kept as a prefix test rather than a
/// schema whitelist so a tracker that adds a type cannot silently hide issues.
const ISSUE_PREFIX: &str = "issue/";

/// The short id from an entity key.
///
/// A Marbles entity id is `<type>/<json key>`, where the key is the record's
/// natural key as a JSON array — `issue/["mb-7"]`. The first element is the id
/// a person reads. A key this build cannot parse falls back to the whole tail
/// rather than to nothing, because a card with a strange label is still
/// better than a card with no name.
fn id_from_key(id: &str) -> String {
    let tail = &id[ISSUE_PREFIX.len()..];
    match serde_json::from_str::<Vec<String>>(tail) {
        Ok(key) => key.into_iter().next().unwrap_or_else(|| tail.to_owned()),
        Err(_) => tail.to_owned(),
    }
}

/// Decode the `issue` records out of one scan of the book.
///
/// Deleted entries and bodies that are not an issue-shaped object are skipped:
/// a board is a view, and a view that refuses to draw because one row is odd is
/// worse than a view that draws the rows it understands.
#[must_use]
pub fn issues(entries: &[(String, Vec<u8>)]) -> Vec<Issue> {
    let mut out = Vec::new();
    for (id, body) in entries {
        if !id.starts_with(ISSUE_PREFIX) {
            continue;
        }
        let Ok(issue) = serde_json::from_slice::<Issue>(body) else {
            continue;
        };
        // The entity id is the authority on identity; a body that disagrees is
        // still keyed by what the store says.
        let mut issue = issue;
        if issue.id.is_empty() {
            issue.id = id_from_key(id);
        }
        out.push(issue);
    }
    // Newest first inside a column, and stable across reloads.
    out.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// The clock the view uses.
///
/// A Spore has no clock grant, and a chart that moved because a guest guessed
/// the hour would disagree with the store it is drawing. The newest record the
/// book holds is the latest moment anything happened; that is what "now" means
/// here, and it is what makes a capture reproducible.
#[must_use]
pub fn latest(issues: &[Issue]) -> i64 {
    issues
        .iter()
        .map(|issue| issue.updated_at.max(issue.created_at))
        .max()
        .unwrap_or(0)
}

/// Whether one issue answers a search.
///
/// Case-insensitive over the fields a person would type: the short id, the
/// title, a label, the project, or who holds it. An empty query matches
/// everything, which is what an empty box means.
#[must_use]
pub fn matches(issue: &Issue, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    if issue.id.to_lowercase().contains(&needle)
        || issue.title.to_lowercase().contains(&needle)
        || issue.project.to_lowercase().contains(&needle)
    {
        return true;
    }
    if issue
        .labels
        .iter()
        .any(|label| label.to_lowercase().contains(&needle))
    {
        return true;
    }
    issue
        .assignee
        .as_deref()
        .is_some_and(|who| who.to_lowercase().contains(&needle))
}

/// A card: the bounded projection a column actually draws.
///
/// A full issue carries a description, evidence and a dependency list. A card
/// that carried those would make the column as expensive as the detail view.
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

/// One column of the board.
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

/// Group the matching issues into the six status columns.
///
/// A status this build has never heard of still gets a column, appended after
/// the six, rather than being dropped: an issue that exists but is not drawn is
/// exactly the failure a board is meant to prevent.
#[must_use]
pub fn columns(issues: &[Issue], query: &str) -> Vec<Column> {
    let mut columns: Vec<Column> = STATUSES
        .iter()
        .map(|status| Column {
            status: (*status).to_owned(),
            title: column_title(status),
            cards: Vec::new(),
        })
        .collect();

    for issue in issues.iter().filter(|issue| matches(issue, query)) {
        let at = match columns.iter().position(|c| c.status == issue.status) {
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

/// A burndown over a window ending at `now`.
#[derive(Debug, Clone)]
pub struct Burndown {
    pub days: Vec<Day>,
    pub scope: i64,
    pub done: i64,
    pub remaining: i64,
    /// Closed issues per day across the window, when anything closed at all.
    pub burn_per_day: Option<f64>,
    /// Remaining divided by the burn rate.
    pub projected_days_to_zero: Option<f64>,
}

impl Burndown {
    /// The tallest value on the chart, so a caller can scale without rescanning.
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

/// Daily remaining-work series over the last `days` days.
///
/// This is the same arithmetic `marbles`' own report uses, applied to the
/// subset the search admits: the ideal line burns the whole scope evenly across
/// the window, and a day's `scope` is the work that existed by that day's end.
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
/// Howard Hinnant's `civil_from_days`, which is the whole of what a day label
/// needs and saves a calendar dependency in a component that has no clock.
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

/// `YYYY-MM-DD` for the day an instant falls in, in UTC.
#[must_use]
pub fn date_label(at: i64) -> String {
    let (y, m, d) = civil_date(at.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(id: &str, status: &str, created: i64, closed: Option<i64>) -> Issue {
        Issue {
            id: id.to_owned(),
            title: format!("work {id}"),
            status: status.to_owned(),
            created_at: created,
            closed_at: closed,
            ..Issue::default()
        }
    }

    #[test]
    fn records_of_other_types_are_not_issues() {
        let entries = vec![
            (
                "issue/[\"a\"]".to_string(),
                br#"{"id":"a","title":"A"}"#.to_vec(),
            ),
            ("project/[\"p\"]".to_string(), br#"{"slug":"p"}"#.to_vec()),
            ("dependency/[\"a\",\"b\"]".to_string(), br#"{ }"#.to_vec()),
        ];
        let issues = issues(&entries);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, "a");
    }

    #[test]
    fn a_body_without_an_id_takes_the_one_from_the_key() {
        let entries = vec![("issue/[\"mb-7\"]".to_string(), br#"{"title":"T"}"#.to_vec())];
        assert_eq!(issues(&entries)[0].id, "mb-7");
    }

    #[test]
    fn an_undecodable_body_is_skipped_not_fatal() {
        let entries = vec![
            ("issue/[\"a\"]".to_string(), b"not json".to_vec()),
            ("issue/[\"b\"]".to_string(), br#"{"id":"b"}"#.to_vec()),
        ];
        assert_eq!(issues(&entries).len(), 1);
    }

    #[test]
    fn search_covers_id_title_label_project_and_assignee() {
        let mut a = issue("mb-1", "open", 0, None);
        a.title = "Fix the parser".into();
        a.project = "marbles".into();
        a.labels = vec!["bug".into()];
        a.assignee = Some("ada".into());

        assert!(matches(&a, "mb-1"));
        assert!(matches(&a, "PARSER"));
        assert!(matches(&a, "marbles"));
        assert!(matches(&a, "bug"));
        assert!(matches(&a, "ada"));
        assert!(matches(&a, "   "));
        assert!(!matches(&a, "zebra"));
    }

    #[test]
    fn columns_are_in_status_order_and_keep_unknown_statuses() {
        let issues = vec![
            issue("a", "open", 0, None),
            issue("b", "closed", 0, Some(1)),
            issue("c", "blocked", 0, None),
            issue("d", "waiting", 0, None),
        ];
        let columns = columns(&issues, "");
        let names: Vec<&str> = columns.iter().map(|c| c.status.as_str()).collect();
        assert_eq!(&names[..3], &["open", "in_progress", "review"]);
        assert!(names.contains(&"waiting"));
        assert_eq!(
            columns.iter().find(|c| c.status == "open").unwrap().count(),
            1
        );
        assert_eq!(columns.len(), 7);
    }

    #[test]
    fn search_narrows_the_columns() {
        let mut a = issue("mb-1", "open", 0, None);
        a.title = "parser".into();
        let b = issue("mb-2", "open", 0, None);
        let columns = columns(&[a, b], "parser");
        assert_eq!(
            columns.iter().find(|c| c.status == "open").unwrap().count(),
            1
        );
    }

    #[test]
    fn burndown_burns_a_three_day_window() {
        let day = 86_400;
        let now = 100 * day;
        // The window is [97d, 100d]. Both created on 98d, so neither exists at
        // the window's first bucket; one closes the next day.
        let issues = vec![
            issue("a", "closed", 98 * day, Some(99 * day)),
            issue("b", "open", 98 * day, None),
        ];
        let burn = burndown(&issues, 3, now);

        assert_eq!(burn.scope, 2);
        assert_eq!(burn.days.len(), 4);
        // At the window's start neither existed yet.
        assert_eq!(burn.days[0].scope, 0);
        assert_eq!(burn.days[0].ideal, 2);
        // By the end both exist and one is done.
        let last = burn.days.last().unwrap();
        assert_eq!(last.scope, 2);
        assert_eq!(last.done, 1);
        assert_eq!(last.remaining, 1);
        assert_eq!(last.ideal, 0);
        assert_eq!(burn.remaining, 1);
        // One close across three days.
        assert!((burn.burn_per_day.unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert!((burn.projected_days_to_zero.unwrap() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn work_created_exactly_at_the_window_start_is_in_scope() {
        // The bucket test is `created_at <= t`, the same rule the tracker's own
        // report uses: a day's scope is the work that existed by its end.
        let day = 86_400;
        let issues = vec![issue("a", "open", 97 * day, None)];
        let burn = burndown(&issues, 3, 100 * day);
        assert_eq!(burn.days[0].scope, 1);
    }

    #[test]
    fn a_window_with_no_closes_has_no_projection() {
        let day = 86_400;
        let issues = vec![issue("a", "open", 99 * day, None)];
        let burn = burndown(&issues, 7, 100 * day);
        assert!(burn.burn_per_day.is_none());
        assert!(burn.projected_days_to_zero.is_none());
        assert_eq!(burn.remaining, 1);
    }

    #[test]
    fn the_ideal_line_falls_from_scope_to_zero() {
        let day = 86_400;
        let now = 10 * day;
        let issues: Vec<Issue> = (0..4)
            .map(|n| issue(&format!("m{n}"), "open", 5 * day, None))
            .collect();
        let burn = burndown(&issues, 4, now);
        assert_eq!(burn.days.first().unwrap().ideal, 4);
        assert_eq!(burn.days.last().unwrap().ideal, 0);
        assert!(burn.peak() >= 4);
    }

    #[test]
    fn a_key_that_is_not_json_still_names_the_card() {
        let entries = vec![("issue/mb-9".to_string(), br#"{}"#.to_vec())];
        assert_eq!(issues(&entries)[0].id, "mb-9");
    }

    #[test]
    fn latest_is_the_newest_moment_in_the_book() {
        let mut a = issue("a", "open", 10, None);
        a.updated_at = 40;
        let b = issue("b", "open", 30, None);
        assert_eq!(latest(&[a, b]), 40);
        assert_eq!(latest(&[]), 0);
    }

    #[test]
    fn dates_are_civil() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(19_000), (2022, 1, 8));
        assert_eq!(date_label(0), "1970-01-01");
    }
}
