//! The rules, and the one function that applies them.
//!
//! [`evaluate`] is a pure function from a [`Snapshot`] to a [`Report`]. It does
//! not read the clock, open a socket, or look at a path, so the answer a shell
//! command prints and the answer a view draws are the same answer, byte for
//! byte, from the same bytes.

use serde::{Deserialize, Serialize};

use crate::check::{Check, Severity, Status};
use crate::date::days_between;
use crate::report::{RepoReport, Report, SkipReason, Skipped};
use crate::snapshot::{RepoState, Snapshot};

/// The thresholds and the scope. Tuning the fleet is editing this, not a check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    /// A repository whose last push is older than this is drifting. 90 days is
    /// one quarter: long enough to be a quiet project rather than a bad week.
    pub stale_days: i64,
    /// Branches per repository before the count is worth a look.
    pub max_branches: u32,
    /// Open pull requests per repository before the count is worth a look.
    pub max_open_pulls: u32,
    /// Whether an archived repository is skipped. Off by default because an
    /// archived repository is a decision somebody already made.
    pub check_archived: bool,
    /// Whether a fork is skipped. A fork is somebody else's repository with a
    /// local ref, and auditing it says more about upstream than about us.
    pub check_forks: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            stale_days: 90,
            max_branches: 50,
            max_open_pulls: 10,
            check_archived: false,
            check_forks: false,
        }
    }
}

/// One check's verdict on one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub check: Check,
    pub severity: Severity,
    #[serde(flatten)]
    pub status: Status,
}

impl Finding {
    fn pass(check: Check) -> Self {
        Self {
            check,
            severity: check.severity(),
            status: Status::Pass,
        }
    }

    fn fail(check: Check, detail: impl Into<String>) -> Self {
        Self {
            check,
            severity: check.severity(),
            status: Status::Fail {
                detail: detail.into(),
            },
        }
    }

    fn unknown(check: Check, detail: impl Into<String>) -> Self {
        Self {
            check,
            severity: check.severity(),
            status: Status::Unknown {
                detail: detail.into(),
            },
        }
    }
}

/// Apply every check to every repository.
#[must_use]
pub fn evaluate(snapshot: &Snapshot) -> Report {
    let mut repos = Vec::new();
    let mut skipped = Vec::new();

    for repo in &snapshot.repos {
        let skip = if repo.archived && !snapshot.policy.check_archived {
            Some(SkipReason::Archived)
        } else if repo.fork && !snapshot.policy.check_forks {
            Some(SkipReason::Fork)
        } else {
            None
        };
        if let Some(reason) = skip {
            skipped.push(Skipped {
                org: repo.org.clone(),
                name: repo.name.clone(),
                reason,
            });
            continue;
        }

        let findings = Check::ALL
            .into_iter()
            .map(|check| match repo.exempt.get(check.id()) {
                Some(reason) => Finding {
                    check,
                    severity: check.severity(),
                    status: Status::Exempt {
                        reason: reason.clone(),
                    },
                },
                None => evaluate_one(
                    check,
                    repo,
                    &snapshot.policy,
                    snapshot.collected_at.as_deref(),
                ),
            })
            .collect();

        repos.push(RepoReport {
            org: repo.org.clone(),
            name: repo.name.clone(),
            findings,
        });
    }

    // A stable order is not cosmetic: a diff between two reports is how a fleet
    // reads, and the forge's own ordering is by name within an owner only.
    repos.sort_by_key(|left| left.slug());
    skipped.sort_by_key(|left| left.slug());

    Report::new(
        snapshot.schema,
        snapshot.collected_at.clone(),
        snapshot.policy.clone(),
        snapshot.orgs(),
        repos,
        skipped,
    )
}

fn evaluate_one(
    check: Check,
    repo: &RepoState,
    policy: &Policy,
    collected_at: Option<&str>,
) -> Finding {
    match check {
        Check::License => match &repo.license {
            Some(spdx) if !spdx.trim().is_empty() => Finding::pass(check),
            _ => Finding::fail(check, "no licence file the forge can name"),
        },
        Check::Ci => match repo.workflows.len() {
            0 => Finding::fail(check, "no .github/workflows entry"),
            _ => Finding::pass(check),
        },
        Check::CiGreen => match &repo.latest_run {
            None => Finding::unknown(check, "no workflow run was collected"),
            Some(run) => match run.conclusion.as_str() {
                "success" => Finding::pass(check),
                "" => Finding::unknown(check, "the last run has not concluded"),
                other => Finding::fail(check, format!("last run concluded {other}")),
            },
        },
        Check::Release => match repo.releases {
            0 => Finding::fail(check, "no releases"),
            _ => Finding::pass(check),
        },
        Check::Description => match repo.description.as_deref().map(str::trim) {
            Some(description) if !description.is_empty() => Finding::pass(check),
            _ => Finding::fail(check, "the forge description is empty"),
        },
        Check::DocsLinks => match &repo.links {
            None => Finding::unknown(check, "no link check was collected"),
            Some(links) => {
                let broken: Vec<&crate::snapshot::Link> =
                    links.iter().filter(|link| !link_ok(link)).collect();
                if broken.is_empty() {
                    Finding::pass(check)
                } else {
                    let sample: Vec<String> = broken
                        .iter()
                        .take(3)
                        .map(|link| match (link.status, &link.error) {
                            (Some(status), _) => format!("{} -> {status}", link.url),
                            (None, Some(error)) => format!("{} -> {error}", link.url),
                            (None, None) => link.url.clone(),
                        })
                        .collect();
                    Finding::fail(
                        check,
                        format!(
                            "{} of {} do not resolve: {}",
                            broken.len(),
                            links.len(),
                            sample.join("; ")
                        ),
                    )
                }
            }
        },
        Check::Active => match (collected_at, repo.pushed_at.as_deref()) {
            (None, _) => Finding::unknown(check, "the snapshot has no collected_at"),
            (_, None) => Finding::unknown(check, "no push date was collected"),
            (Some(collected_at), Some(pushed_at)) => match days_between(pushed_at, collected_at) {
                None => Finding::unknown(check, "the push date is unreadable"),
                Some(age) if age > policy.stale_days => Finding::fail(
                    check,
                    format!("last push was {age} days ago, limit {}", policy.stale_days),
                ),
                Some(_) => Finding::pass(check),
            },
        },
        Check::Branches => {
            if repo.branches > policy.max_branches {
                Finding::fail(
                    check,
                    format!("{} branches, limit {}", repo.branches, policy.max_branches),
                )
            } else {
                Finding::pass(check)
            }
        }
        Check::OpenPulls => {
            if repo.open_pulls > policy.max_open_pulls {
                Finding::fail(
                    check,
                    format!(
                        "{} open pull requests, limit {}",
                        repo.open_pulls, policy.max_open_pulls
                    ),
                )
            } else {
                Finding::pass(check)
            }
        }
    }
}

fn link_ok(link: &crate::snapshot::Link) -> bool {
    link.error.is_none()
        && link
            .status
            .is_some_and(|status| (200..400).contains(&status))
}

#[cfg(test)]
mod tests {
    use super::{Finding, Policy, evaluate};
    use crate::check::{Check, Status};
    use crate::report::SkipReason;
    use crate::snapshot::{Link, RepoState, Run, Snapshot};

    fn repo() -> RepoState {
        RepoState {
            org: "fpl".into(),
            name: "bob".into(),
            pushed_at: Some("2026-11-17T00:00:00Z".into()),
            license: Some("MIT".into()),
            description: Some("a build plane".into()),
            releases: 3,
            branches: 4,
            workflows: vec!["ci.yml".into()],
            ..RepoState::default()
        }
    }

    fn snapshot(repos: Vec<RepoState>) -> Snapshot {
        Snapshot {
            schema: crate::SCHEMA,
            collected_at: Some("2026-11-17T12:00:00Z".into()),
            policy: Policy::default(),
            repos,
        }
    }

    fn find(report: &crate::Report, check: Check) -> &Finding {
        report.repos[0]
            .findings
            .iter()
            .find(|finding| finding.check == check)
            .expect("every check is evaluated for every repository")
    }

    #[test]
    fn a_healthy_repository_passes_everything_it_has_evidence_for() {
        let report = evaluate(&snapshot(vec![repo()]));
        assert_eq!(find(&report, Check::License).status, Status::Pass);
        assert_eq!(find(&report, Check::Ci).status, Status::Pass);
        assert_eq!(find(&report, Check::Release).status, Status::Pass);
        assert_eq!(find(&report, Check::Active).status, Status::Pass);
        // No evidence was collected for these two, so they cannot pass.
        assert!(find(&report, Check::CiGreen).status.is_unknown());
        assert!(find(&report, Check::DocsLinks).status.is_unknown());
    }

    #[test]
    fn an_empty_repository_fails_the_critical_three() {
        let bare = RepoState {
            pushed_at: Some("2026-11-17T00:00:00Z".into()),
            ..RepoState {
                org: "fpl".into(),
                name: "bare".into(),
                ..RepoState::default()
            }
        };
        let report = evaluate(&snapshot(vec![bare]));
        assert!(find(&report, Check::License).status.is_fail());
        assert!(find(&report, Check::Ci).status.is_fail());
        assert!(find(&report, Check::Release).status.is_fail());
        assert!(find(&report, Check::Description).status.is_fail());
    }

    #[test]
    fn a_red_default_branch_fails_and_an_unconcluded_run_does_not() {
        let mut red = repo();
        red.latest_run = Some(Run {
            workflow: "ci".into(),
            conclusion: "failure".into(),
            at: None,
        });
        let report = evaluate(&snapshot(vec![red]));
        assert!(find(&report, Check::CiGreen).status.is_fail());

        let mut running = repo();
        running.latest_run = Some(Run {
            workflow: "ci".into(),
            conclusion: String::new(),
            at: None,
        });
        let report = evaluate(&snapshot(vec![running]));
        assert!(find(&report, Check::CiGreen).status.is_unknown());
    }

    #[test]
    fn staleness_needs_a_reference_time_and_says_so_without_one() {
        let mut snapshot = snapshot(vec![repo()]);
        snapshot.collected_at = None;
        let report = evaluate(&snapshot);
        assert!(find(&report, Check::Active).status.is_unknown());

        snapshot.collected_at = Some("2027-03-01T00:00:00Z".into());
        let report = evaluate(&snapshot);
        assert!(find(&report, Check::Active).status.is_fail());
    }

    #[test]
    fn branch_and_pull_counts_are_thresholds_not_booleans() {
        let mut busy = repo();
        busy.branches = 51;
        busy.open_pulls = 11;
        let report = evaluate(&snapshot(vec![busy]));
        assert!(find(&report, Check::Branches).status.is_fail());
        assert!(find(&report, Check::OpenPulls).status.is_fail());
    }

    #[test]
    fn a_broken_link_fails_and_a_redirected_one_passes() {
        let mut linked = repo();
        linked.links = Some(vec![
            Link {
                url: "https://a.example".into(),
                status: Some(301),
                error: None,
            },
            Link {
                url: "https://b.example".into(),
                status: Some(404),
                error: None,
            },
        ]);
        let report = evaluate(&snapshot(vec![linked]));
        match &find(&report, Check::DocsLinks).status {
            Status::Fail { detail } => assert!(detail.contains("1 of 2"), "{detail}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn an_exemption_is_visible_rather_than_silent() {
        let mut exempt = repo();
        exempt.license = None;
        exempt
            .exempt
            .insert("license".into(), "proprietary, see legal".into());
        let report = evaluate(&snapshot(vec![exempt]));
        assert_eq!(
            find(&report, Check::License).status,
            Status::Exempt {
                reason: "proprietary, see legal".into()
            }
        );
    }

    #[test]
    fn archived_and_forked_repositories_are_counted_not_scored() {
        let mut archived = repo();
        archived.archived = true;
        let mut fork = repo();
        fork.name = "forked".into();
        fork.fork = true;
        let report = evaluate(&snapshot(vec![archived, fork]));
        assert!(report.repos.is_empty());
        assert_eq!(report.skipped.len(), 2);
        assert_eq!(report.skipped[0].reason, SkipReason::Archived);
        assert_eq!(report.skipped[1].reason, SkipReason::Fork);
    }

    #[test]
    fn the_policy_can_widen_the_scope() {
        let mut fork = repo();
        fork.fork = true;
        let mut snapshot = snapshot(vec![fork]);
        snapshot.policy.check_forks = true;
        let report = evaluate(&snapshot);
        assert_eq!(report.repos.len(), 1);
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn repositories_come_out_sorted_so_two_reports_diff_cleanly() {
        let mut second = repo();
        second.name = "aaa".into();
        let report = evaluate(&snapshot(vec![repo(), second]));
        assert_eq!(report.repos[0].slug(), "fpl/aaa");
        assert_eq!(report.repos[1].slug(), "fpl/bob");
    }
}
