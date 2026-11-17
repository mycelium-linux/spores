//! The vocabulary of checks, and what each one is allowed to fail on.
//!
//! Three levels, and the middle one is the point:
//!
//! - **pass** — the evidence says yes;
//! - **fail** — the evidence says no;
//! - **unknown** — there is no evidence.
//!
//! Unknown is not pass. A repo whose CI state was never collected is not green,
//! it is unmeasured, and a report that silently calls that green is how a fleet
//! rots while the dashboard stays calm. `--strict` is the caller saying it would
//! rather see the noise than the lie.

use serde::{Deserialize, Serialize};

/// How much a finding matters.
///
/// Ordered so that `>=` reads as "at least this bad": advisory is the floor, so
/// a `--fail-on critical` gate is `severity >= Critical` and a
/// `--fail-on advisory` gate is `severity >= Advisory`, which is everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// It works, and it is drifting.
    Advisory,
    /// The repository is not doing the thing at all.
    Critical,
}

impl Severity {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Advisory => "advisory",
        }
    }
}

/// One check's answer for one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    /// The evidence says yes.
    Pass,
    /// The evidence says no; the detail says what was missing.
    Fail { detail: String },
    /// No evidence was collected, so this check cannot say.
    Unknown { detail: String },
    /// Somebody decided this repository does not need it.
    Exempt { reason: String },
}

impl Status {
    #[must_use]
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }

    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }
}

/// A question the fleet is asked, once, in one place.
///
/// The list is code and the thresholds are policy: a new question is a new
/// check, and tuning an old one is a number in [`crate::Policy`]. Keeping the
/// vocabulary here is what stops a view from inventing a tenth check that the
/// command line has never heard of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Check {
    /// A licence is declared.
    License,
    /// There is CI at all.
    Ci,
    /// The last run on the default branch passed.
    CiGreen,
    /// The repository has shipped something.
    Release,
    /// Somebody wrote one sentence about it.
    Description,
    /// The documentation links in the repository resolve.
    DocsLinks,
    /// It was pushed to recently.
    Active,
    /// Branches are not accumulating.
    Branches,
    /// Pull requests are not accumulating.
    OpenPulls,
}

impl Check {
    /// Every check, in the order a report prints them.
    pub const ALL: [Self; 9] = [
        Self::License,
        Self::Ci,
        Self::CiGreen,
        Self::Release,
        Self::Description,
        Self::DocsLinks,
        Self::Active,
        Self::Branches,
        Self::OpenPulls,
    ];

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::License => "license",
            Self::Ci => "ci",
            Self::CiGreen => "ci-green",
            Self::Release => "release",
            Self::Description => "description",
            Self::DocsLinks => "docs-links",
            Self::Active => "active",
            Self::Branches => "branches",
            Self::OpenPulls => "open-pulls",
        }
    }

    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            Self::License | Self::Ci | Self::CiGreen => Severity::Critical,
            Self::Release
            | Self::Description
            | Self::DocsLinks
            | Self::Active
            | Self::Branches
            | Self::OpenPulls => Severity::Advisory,
        }
    }

    /// The question, phrased so that a pass reads as an answer.
    #[must_use]
    pub fn question(self) -> &'static str {
        match self {
            Self::License => "does the repository declare a licence",
            Self::Ci => "is there a workflow under .github/workflows",
            Self::CiGreen => "did the last run on the default branch pass",
            Self::Release => "has it ever cut a release",
            Self::Description => "does the forge description say what it is",
            Self::DocsLinks => "do the documentation links resolve",
            Self::Active => "was it pushed to inside the staleness window",
            Self::Branches => "are branches under the limit",
            Self::OpenPulls => "are open pull requests under the limit",
        }
    }

    /// What a reviewer should do about a failure.
    #[must_use]
    pub fn remedy(self) -> &'static str {
        match self {
            Self::License => "add LICENSE (and set it in spore.toml / Cargo.toml / package.json)",
            Self::Ci => "add .github/workflows/ci.yml, or adopt the fleet workflow",
            Self::CiGreen => "fix the failing run, or remove the workflow that fails",
            Self::Release => "cut a release, or delete the release workflow if it is unused",
            Self::Description => "set the repository description",
            Self::DocsLinks => "fix or remove the dead link",
            Self::Active => "archive it, or push the work that is sitting locally",
            Self::Branches => "delete merged branches",
            Self::OpenPulls => "land it, close it, or mark it draft with a reason",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Check, Severity};

    #[test]
    fn ids_are_unique_and_stable() {
        let mut ids: Vec<&str> = Check::ALL.iter().map(|check| check.id()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "two checks share an id");
    }

    #[test]
    fn every_check_answers_and_has_a_remedy() {
        for check in Check::ALL {
            assert!(!check.question().is_empty(), "{check:?} has no question");
            assert!(!check.remedy().is_empty(), "{check:?} has no remedy");
        }
    }

    #[test]
    fn the_three_that_stop_a_repository_are_critical() {
        assert_eq!(Check::License.severity(), Severity::Critical);
        assert_eq!(Check::Ci.severity(), Severity::Critical);
        assert_eq!(Check::CiGreen.severity(), Severity::Critical);
        assert_eq!(Check::Branches.severity(), Severity::Advisory);
    }
}
