//! The report: what the checks found, and how to read it on a terminal.
//!
//! A report is the only document a consumer needs. The command line prints one,
//! a view draws one, and both see the same counts because the summary is
//! computed once here rather than twice downstream.

use serde::{Deserialize, Serialize};

use crate::check::{Check, Severity, Status};
use crate::policy::{Finding, Policy};

/// Why a repository was not scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    Archived,
    Fork,
}

impl SkipReason {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Archived => "archived",
            Self::Fork => "fork",
        }
    }
}

/// One repository that was skipped, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub org: String,
    pub name: String,
    pub reason: SkipReason,
}

impl Skipped {
    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.org, self.name)
    }
}

/// One repository's findings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoReport {
    pub org: String,
    pub name: String,
    pub findings: Vec<Finding>,
}

impl RepoReport {
    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.org, self.name)
    }

    pub fn fails(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.status.is_fail())
    }

    /// The worst failing severity, or `None` when nothing failed.
    #[must_use]
    pub fn worst_fail(&self) -> Option<Severity> {
        self.fails().map(|finding| finding.severity).max()
    }

    #[must_use]
    pub fn has_unknown(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.status.is_unknown())
    }

    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.worst_fail().is_none() && !self.has_unknown()
    }
}

/// How one check did across the fleet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSummary {
    pub check: Check,
    pub severity: Severity,
    pub pass: usize,
    pub fail: usize,
    pub unknown: usize,
    pub exempt: usize,
}

/// The fleet in numbers, so a view renders a heading instead of counting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Repositories that were scored.
    pub checked: usize,
    /// Repositories that were skipped, by reason.
    pub skipped: usize,
    pub archived: usize,
    pub forks: usize,
    /// Nothing failed and nothing was left unknown.
    pub clean: usize,
    /// At least one critical check failed.
    pub critical: usize,
    /// Nothing critical failed, and something advisory did.
    pub advisory: usize,
    /// At least one check could not answer, in any repository.
    pub unknown: usize,
    pub by_check: Vec<CheckSummary>,
}

impl Summary {
    fn new(repos: &[RepoReport], skipped: &[Skipped]) -> Self {
        let mut by_check: Vec<CheckSummary> = Check::ALL
            .into_iter()
            .map(|check| CheckSummary {
                check,
                severity: check.severity(),
                pass: 0,
                fail: 0,
                unknown: 0,
                exempt: 0,
            })
            .collect();

        let (mut clean, mut critical, mut advisory, mut unknown) = (0, 0, 0, 0);
        for repo in repos {
            match repo.worst_fail() {
                Some(Severity::Critical) => critical += 1,
                Some(Severity::Advisory) => advisory += 1,
                None => {}
            }
            if repo.has_unknown() {
                unknown += 1;
            }
            if repo.is_clean() {
                clean += 1;
            }
            for finding in &repo.findings {
                let Some(entry) = by_check
                    .iter_mut()
                    .find(|entry| entry.check == finding.check)
                else {
                    continue;
                };
                match finding.status {
                    Status::Pass => entry.pass += 1,
                    Status::Fail { .. } => entry.fail += 1,
                    Status::Unknown { .. } => entry.unknown += 1,
                    Status::Exempt { .. } => entry.exempt += 1,
                }
            }
        }

        Self {
            checked: repos.len(),
            skipped: skipped.len(),
            archived: skipped
                .iter()
                .filter(|s| s.reason == SkipReason::Archived)
                .count(),
            forks: skipped
                .iter()
                .filter(|s| s.reason == SkipReason::Fork)
                .count(),
            clean,
            critical,
            advisory,
            unknown,
            by_check,
        }
    }
}

/// Every repository's findings, plus the rollup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected_at: Option<String>,
    pub policy: Policy,
    pub orgs: Vec<String>,
    pub summary: Summary,
    pub repos: Vec<RepoReport>,
    pub skipped: Vec<Skipped>,
}

impl Report {
    #[must_use]
    pub fn new(
        schema: u32,
        collected_at: Option<String>,
        policy: Policy,
        orgs: Vec<String>,
        repos: Vec<RepoReport>,
        skipped: Vec<Skipped>,
    ) -> Self {
        let summary = Summary::new(&repos, &skipped);
        Self {
            schema,
            collected_at,
            policy,
            orgs,
            summary,
            repos,
            skipped,
        }
    }

    /// Repositories with findings, worst first.
    ///
    /// Critical before advisory, then more failures before fewer, then by name.
    /// The last key is what makes two runs comparable: a report that reorders
    /// itself between collections cannot be diffed.
    #[must_use]
    pub fn worst(&self, include_unknown: bool) -> Vec<&RepoReport> {
        let mut failing: Vec<&RepoReport> = self
            .repos
            .iter()
            .filter(|repo| repo.worst_fail().is_some() || (include_unknown && repo.has_unknown()))
            .collect();
        failing.sort_by(|left, right| {
            right
                .worst_fail()
                .cmp(&left.worst_fail())
                .then_with(|| right.fails().count().cmp(&left.fails().count()))
                .then_with(|| left.slug().cmp(&right.slug()))
        });
        failing
    }

    /// Repositories with no failing check, whether or not anything is unknown.
    #[must_use]
    pub fn failing_count(&self) -> usize {
        self.repos
            .iter()
            .filter(|repo| repo.worst_fail().is_some())
            .count()
    }
}

/// Render the report for a terminal. `limit` caps the per-repository listing;
/// the summary is always whole, because that is the part a person reads first.
#[must_use]
pub fn render_text(report: &Report) -> String {
    render_text_with_limit(report, None)
}

/// Render the report for a terminal, listing at most `limit` repositories.
#[must_use]
pub fn render_text_with_limit(report: &Report, limit: Option<usize>) -> String {
    let mut out = String::new();
    let orgs = if report.orgs.is_empty() {
        "(no owner)".to_string()
    } else {
        report.orgs.join(", ")
    };
    out.push_str(&format!("warden: {orgs}\n"));

    let summary = &report.summary;
    let mut scope = format!("{} repositories", summary.checked);
    if summary.skipped > 0 {
        let mut parts = Vec::new();
        if summary.archived > 0 {
            parts.push(format!("{} archived", summary.archived));
        }
        if summary.forks > 0 {
            parts.push(format!("{} forks", summary.forks));
        }
        scope.push_str(&format!(
            " ({} skipped: {})",
            summary.skipped,
            parts.join(", ")
        ));
    }
    if let Some(collected_at) = &report.collected_at {
        scope.push_str(&format!(" · collected {collected_at}"));
    }
    out.push_str(&scope);
    out.push('\n');

    out.push_str(&format!(
        "\n  {} fail a critical check · {} advisory only · {} clean\n",
        summary.critical, summary.advisory, summary.clean
    ));

    out.push_str("\ncheck            fail  unknown   share\n");
    let widest = summary.checked.max(1);
    let mut exempts = 0;
    for entry in &summary.by_check {
        let share = entry.fail as f64 / widest as f64;
        let filled = (share * 24.0).round() as usize;
        let bar: String = "█".repeat(filled);
        let unknown = if entry.unknown > 0 {
            entry.unknown.to_string()
        } else {
            "-".to_string()
        };
        out.push_str(&format!(
            "{:<14} {:>5} {:>8}   {:<24} {:>3}%\n",
            entry.check.id(),
            entry.fail,
            unknown,
            bar,
            (share * 100.0).round() as u32
        ));
        exempts += entry.exempt;
    }
    if exempts > 0 {
        out.push_str(&format!(
            "\n  {exempts} exemptions are in force, each with a written reason.\n  `warden explain <owner/name>` shows them.\n"
        ));
    }
    if summary.unknown > 0 {
        out.push_str(
            "\n  unknown is not a pass: the collector did not supply that evidence.\n  `--strict` fails on it.\n",
        );
    }

    let worst = report.worst(false);
    if worst.is_empty() {
        out.push_str("\nno repository failed a check.\n");
        return out;
    }

    out.push_str(&format!(
        "\n{} repositories with findings, worst first:\n",
        worst.len()
    ));
    let shown = limit.unwrap_or(worst.len()).min(worst.len());
    for repo in worst.iter().take(shown) {
        out.push('\n');
        out.push_str(&format!("  {}\n", repo.slug()));
        for finding in repo.fails() {
            let mark = match finding.severity {
                Severity::Critical => '!',
                Severity::Advisory => '~',
            };
            let Status::Fail { detail } = &finding.status else {
                continue;
            };
            out.push_str(&format!("    {mark} {:<12} {detail}\n", finding.check.id()));
        }
    }
    if shown < worst.len() {
        out.push_str(&format!(
            "\n{} more with findings. `--all` prints them, `--limit N` widens this list.\n",
            worst.len() - shown
        ));
    }
    out
}

/// Render one repository in full: every check, with the remedy for a failure.
#[must_use]
pub fn render_repo(repo: &RepoReport) -> String {
    let mut out = format!("{}\n", repo.slug());
    for finding in &repo.findings {
        let (mark, text) = match &finding.status {
            Status::Pass => ('·', "ok".to_string()),
            Status::Fail { detail } => ('!', detail.clone()),
            Status::Unknown { detail } => ('?', detail.clone()),
            Status::Exempt { reason } => ('-', format!("exempt: {reason}")),
        };
        out.push_str(&format!(
            "  {mark} {:<12} {:<10} {text}\n",
            finding.check.id(),
            finding.severity.id()
        ));
        if finding.status.is_fail() {
            out.push_str(&format!("      → {}\n", finding.check.remedy()));
        }
    }
    out
}

/// Render the report as JSON, the shape another tool reads.
pub fn render_json(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

#[cfg(test)]
mod tests {
    use super::{RepoReport, Report, SkipReason, Skipped, render_repo, render_text};
    use crate::check::{Check, Status};
    use crate::policy::{Finding, Policy};

    fn finding(check: Check, status: Status) -> Finding {
        Finding {
            check,
            severity: check.severity(),
            status,
        }
    }

    fn report() -> Report {
        let repos = vec![
            RepoReport {
                org: "fpl".into(),
                name: "bob".into(),
                findings: vec![
                    finding(
                        Check::License,
                        Status::Fail {
                            detail: "no licence".into(),
                        },
                    ),
                    finding(Check::Ci, Status::Pass),
                ],
            },
            RepoReport {
                org: "fpl".into(),
                name: "fab".into(),
                findings: vec![
                    finding(Check::License, Status::Pass),
                    finding(Check::Ci, Status::Pass),
                    finding(
                        Check::Release,
                        Status::Fail {
                            detail: "no releases".into(),
                        },
                    ),
                    finding(
                        Check::DocsLinks,
                        Status::Unknown {
                            detail: "not collected".into(),
                        },
                    ),
                ],
            },
        ];
        let skipped = vec![Skipped {
            org: "fpl".into(),
            name: "old".into(),
            reason: SkipReason::Archived,
        }];
        Report::new(
            crate::SCHEMA,
            Some("2026-11-17T12:00:00Z".into()),
            Policy::default(),
            vec!["fpl".into()],
            repos,
            skipped,
        )
    }

    #[test]
    fn the_summary_counts_each_repository_once() {
        let summary = report().summary;
        assert_eq!(summary.checked, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.archived, 1);
        assert_eq!(summary.critical, 1);
        assert_eq!(summary.advisory, 1);
        assert_eq!(summary.clean, 0);
        assert_eq!(summary.unknown, 1);
    }

    #[test]
    fn per_check_counts_add_up_to_the_scope() {
        // Every check is present in a report `evaluate` produced, so the four
        // buckets of each check account for every repository.
        let report = crate::evaluate(&crate::Snapshot {
            schema: crate::SCHEMA,
            collected_at: Some("2026-11-17T12:00:00Z".into()),
            policy: Policy::default(),
            repos: vec![
                crate::RepoState {
                    org: "fpl".into(),
                    name: "a".into(),
                    ..crate::RepoState::default()
                },
                crate::RepoState {
                    org: "fpl".into(),
                    name: "b".into(),
                    license: Some("MIT".into()),
                    ..crate::RepoState::default()
                },
            ],
        });
        for entry in &report.summary.by_check {
            assert_eq!(
                entry.pass + entry.fail + entry.unknown + entry.exempt,
                report.summary.checked,
                "{} does not account for every repository",
                entry.check.id()
            );
        }
    }

    #[test]
    fn critical_repositories_sort_before_advisory_ones() {
        let report = report();
        let worst = report.worst(false);
        assert_eq!(worst.len(), 2);
        assert_eq!(worst[0].slug(), "fpl/bob");
        assert_eq!(worst[1].slug(), "fpl/fab");
    }

    #[test]
    fn the_text_says_what_it_looked_at_and_what_it_found() {
        let text = render_text(&report());
        assert!(text.contains("warden: fpl"), "{text}");
        assert!(text.contains("2 repositories"), "{text}");
        assert!(text.contains("1 skipped: 1 archived"), "{text}");
        assert!(text.contains("fpl/bob"), "{text}");
        assert!(text.contains("no licence"), "{text}");
        assert!(text.contains("2026-11-17T12:00:00Z"), "{text}");
    }

    #[test]
    fn a_repository_explanation_carries_the_remedy() {
        let report = report();
        let text = render_repo(&report.repos[0]);
        assert!(text.contains("no licence"), "{text}");
        assert!(text.contains("add LICENSE"), "{text}");
    }

    #[test]
    fn json_round_trips_so_a_view_reads_what_the_command_wrote() {
        let report = report();
        let json = super::render_json(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }

    #[test]
    fn a_clean_fleet_says_so_instead_of_printing_an_empty_table() {
        let repos = vec![RepoReport {
            org: "fpl".into(),
            name: "bob".into(),
            findings: vec![finding(Check::License, Status::Pass)],
        }];
        let report = Report::new(
            crate::SCHEMA,
            None,
            Policy::default(),
            vec!["fpl".into()],
            repos,
            vec![],
        );
        let text = render_text(&report);
        assert!(text.contains("no repository failed a check"), "{text}");
        assert_eq!(report.summary.clean, 1);
    }
}
