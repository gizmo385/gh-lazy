//! `cargo xtask release-plan`: decides whether a run of the release workflow (`.github/workflows/release.yaml`, #203)
//! releases anything, and which tag and version, before any build starts.
//!
//! [`plan`] is pure logic over [`ReleaseInputs`], so every rule is unit-tested. [`run`] gathers the inputs
//! (`cargo metadata --no-deps`, `git tag --list v*`, `git merge-base --is-ancestor <sha> origin/main` and
//! `GITHUB_REPOSITORY`) and prints the plan as `key=value` lines for `$GITHUB_OUTPUT`, the same pattern as
//! `cargo xtask parity --plan`. A human-readable line goes to stderr, since stdout lands in the output file.
//!
//! The rules:
//!
//! - A push to `main` releases `v<workspace version>`, unless that tag already exists (a `Cargo.toml` change that
//!   isn't a version bump). The version must be greater than the newest stable `v*` tag.
//! - A pushed tag must be a prerelease of the workspace version, `v<version>-<suffix>`: the rehearsal path. Stable
//!   tags are only ever created by the workflow, from `main`.
//! - `workflow_dispatch` is a push to `main`, and the commit must be on `main`.
//! - Every stable plan needs the commit on `main`, so `rust-main` can't make a stable release before the cutover.
//! - Crates are published only for a stable release in `gizmo385/gh-lazy`.
//! - The internal crates' version requirements must pin the workspace version exactly (`=X.Y.Z`, #204).

use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::process::{Command, ExitCode};

use serde_json::Value;

/// The only repository that publishes crates and posts to Discord; forks rehearse without them.
pub const UPSTREAM_REPOSITORY: &str = "gizmo385/gh-lazy";

/// The package whose version is the release version. Every workspace package inherits the workspace version.
const APP_PACKAGE: &str = "lazy-github";

#[derive(Debug, clap::Args)]
pub struct ReleasePlanArgs {
    /// The event that started the workflow (`$GITHUB_EVENT_NAME`).
    #[arg(long, value_enum)]
    pub event: Event,
    /// The ref the workflow runs on (`$GITHUB_REF`), such as `refs/heads/main` or `refs/tags/v1.0.0-rc.1`.
    #[arg(long = "ref")]
    pub git_ref: String,
    /// The commit the workflow runs on (`$GITHUB_SHA`).
    #[arg(long)]
    pub sha: String,
    /// The `owner/name` repository [default: `$GITHUB_REPOSITORY`, or none].
    #[arg(long)]
    pub repository: Option<String>,
    /// The ref a stable release's commit must be on.
    #[arg(long, default_value = "origin/main")]
    pub main: String,
}

/// The workflow events the release workflow runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Event {
    #[value(name = "push")]
    Push,
    #[value(name = "workflow_dispatch")]
    WorkflowDispatch,
}

/// A plain `MAJOR.MINOR.PATCH` version. Release versions never carry a prerelease or build suffix: a prerelease is
/// a tag suffix on top of the workspace version, not a workspace version of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parses `X.Y.Z` with no leading zeros (semver's rule), and nothing else.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut number = || {
            let part = parts.next()?;
            let leading_zero = part.len() > 1 && part.starts_with('0');
            if part.is_empty() || leading_zero || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let version = Self {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One workspace package's normal or build dependency on another workspace package, as `cargo metadata` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalDep {
    /// The package that has the dependency.
    pub package: String,
    /// The workspace package it depends on.
    pub dependency: String,
    /// Its version requirement: `*` for a path-only dependency, `=X.Y.Z` once pinned (#204).
    pub requirement: String,
}

/// Everything [`plan`] decides from.
#[derive(Debug, Clone)]
pub struct ReleaseInputs {
    pub event: Event,
    pub git_ref: String,
    pub workspace_version: Version,
    pub internal_deps: Vec<InternalDep>,
    pub existing_tags: Vec<String>,
    /// Whether the commit is an ancestor of (or is) `origin/main`.
    pub sha_on_main: bool,
    /// `owner/name`, empty when unknown.
    pub repository: String,
}

/// What the workflow does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Nothing to release; the workflow's later jobs are skipped.
    Skip { reason: String },
    Release {
        tag: String,
        version: Version,
        prerelease: bool,
        publish_crates: bool,
    },
}

impl Plan {
    /// The `key=value` lines for `$GITHUB_OUTPUT`. A skip still writes every key, so expressions never see a
    /// missing output.
    pub fn outputs(&self) -> String {
        match self {
            Plan::Skip { .. } => "release=false\nversion=\ntag=\nprerelease=false\npublish_crates=false\n".to_owned(),
            Plan::Release {
                tag,
                version,
                prerelease,
                publish_crates,
            } => format!(
                "release=true\nversion={version}\ntag={tag}\nprerelease={prerelease}\npublish_crates={publish_crates}\n"
            ),
        }
    }

    /// One line for the log.
    pub fn summary(&self) -> String {
        match self {
            Plan::Skip { reason } => format!("No release: {reason}."),
            Plan::Release {
                tag,
                prerelease,
                publish_crates,
                ..
            } => {
                let kind = if *prerelease { "prerelease" } else { "release" };
                let crates = if *publish_crates {
                    "publishing the crates"
                } else {
                    "not publishing the crates"
                };
                format!("Plan: {kind} {tag}, {crates}.")
            }
        }
    }
}

/// Why a run can't release. Each is one line, shown as the workflow's error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("v{version} is a stable release, so its commit must be on main")]
    NotOnMain { version: Version },
    #[error(
        "{tag} is a stable tag; the release workflow creates those itself from main, so bump the workspace version \
         on main instead (pushed tags must be prereleases, v<version>-<suffix>)"
    )]
    StableTagPush { tag: String },
    #[error("tag {tag} doesn't match the workspace version {version} (a prerelease tag is v{version}-<suffix>)")]
    TagMismatch { tag: String, version: Version },
    #[error("the release workflow runs on pushes to main and v*-* tags, not {git_ref}")]
    UnexpectedRef { git_ref: String },
    #[error("the workspace version {version} isn't greater than the latest release, v{latest}")]
    NotNewer { version: Version, latest: Version },
    #[error(
        "{package} depends on {dependency} {requirement}, but the workspace version is {version}: pin it as \
         \"={version}\" in [workspace.dependencies]"
    )]
    PinMismatch {
        package: String,
        dependency: String,
        requirement: String,
        version: Version,
    },
}

/// Decides the release. See the module docs for the rules.
pub fn plan(inputs: &ReleaseInputs) -> Result<Plan, PlanError> {
    let version = inputs.workspace_version;
    check_pins(version, &inputs.internal_deps)?;
    match inputs.event {
        Event::Push => {
            if let Some(tag) = inputs.git_ref.strip_prefix("refs/tags/") {
                return prerelease_plan(tag, version);
            }
            if inputs.git_ref != "refs/heads/main" {
                return Err(PlanError::UnexpectedRef {
                    git_ref: inputs.git_ref.clone(),
                });
            }
            stable_plan(inputs)
        }
        // A dispatch may name any branch; the `sha_on_main` check in `stable_plan` is what keeps it to main.
        Event::WorkflowDispatch => stable_plan(inputs),
    }
}

/// Every internal dependency that has a version must pin the workspace version exactly. Path-only ones (`*`) have
/// none to check: cargo refuses to publish those, and dev-dependencies aren't listed here.
pub fn check_pins(version: Version, deps: &[InternalDep]) -> Result<(), PlanError> {
    let expected = format!("={version}");
    match deps
        .iter()
        .find(|dep| dep.requirement != "*" && dep.requirement != expected)
    {
        Some(dep) => Err(PlanError::PinMismatch {
            package: dep.package.clone(),
            dependency: dep.dependency.clone(),
            requirement: dep.requirement.clone(),
            version,
        }),
        None => Ok(()),
    }
}

fn prerelease_plan(tag: &str, version: Version) -> Result<Plan, PlanError> {
    let mismatch = || PlanError::TagMismatch {
        tag: tag.to_owned(),
        version,
    };
    let rest = tag
        .strip_prefix('v')
        .and_then(|rest| rest.strip_prefix(version.to_string().as_str()))
        .ok_or_else(mismatch)?;
    if rest.is_empty() {
        return Err(PlanError::StableTagPush { tag: tag.to_owned() });
    }
    let suffix = rest
        .strip_prefix('-')
        .filter(|suffix| !suffix.is_empty())
        .ok_or_else(mismatch)?;
    // Semver prerelease identifiers: dot-separated, non-empty, ASCII alphanumerics and hyphens.
    let valid = suffix
        .split('.')
        .all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    if !valid {
        return Err(mismatch());
    }
    Ok(Plan::Release {
        tag: tag.to_owned(),
        version,
        prerelease: true,
        publish_crates: false,
    })
}

fn stable_plan(inputs: &ReleaseInputs) -> Result<Plan, PlanError> {
    let version = inputs.workspace_version;
    let tag = format!("v{version}");
    if inputs.existing_tags.contains(&tag) {
        return Ok(Plan::Skip {
            reason: format!("{tag} already exists, so the workspace version wasn't bumped"),
        });
    }
    if !inputs.sha_on_main {
        return Err(PlanError::NotOnMain { version });
    }
    let latest = inputs
        .existing_tags
        .iter()
        .filter_map(|tag| Version::parse(tag.strip_prefix('v')?))
        .max();
    if let Some(latest) = latest
        && version <= latest
    {
        return Err(PlanError::NotNewer { version, latest });
    }
    Ok(Plan::Release {
        tag,
        version,
        prerelease: false,
        publish_crates: inputs.repository == UPSTREAM_REPOSITORY,
    })
}

/// The workspace version and the internal dependencies, from `cargo metadata --no-deps --format-version 1`.
pub fn from_metadata(metadata: &Value) -> Result<(Version, Vec<InternalDep>), String> {
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?;
    let name = |package: &Value| package["name"].as_str().unwrap_or_default().to_owned();
    let members: Vec<String> = packages.iter().map(name).collect();
    let app = packages
        .iter()
        .find(|package| package["name"] == APP_PACKAGE)
        .ok_or_else(|| format!("cargo metadata has no {APP_PACKAGE} package"))?;
    let raw = app["version"].as_str().unwrap_or_default();
    let version = Version::parse(raw)
        .ok_or_else(|| format!("the workspace version {raw:?} isn't a plain MAJOR.MINOR.PATCH version"))?;
    let mut deps = Vec::new();
    for package in packages {
        for dep in package["dependencies"].as_array().into_iter().flatten() {
            let dependency = dep["name"].as_str().unwrap_or_default();
            // `kind` is null for normal dependencies, "build" or "dev" otherwise.
            if dep["kind"] != "dev" && members.iter().any(|member| member == dependency) {
                deps.push(InternalDep {
                    package: name(package),
                    dependency: dependency.to_owned(),
                    requirement: dep["req"].as_str().unwrap_or("*").to_owned(),
                });
            }
        }
    }
    Ok((version, deps))
}

pub fn run(repo: &Path, args: &ReleasePlanArgs) -> ExitCode {
    let result = gather(repo, args).and_then(|inputs| plan(&inputs).map_err(|error| error.to_string()));
    match result {
        Ok(plan) => {
            print!("{}", plan.outputs());
            eprintln!("{}", plan.summary());
            ExitCode::SUCCESS
        }
        Err(error) => {
            // An annotation in Actions puts the reason on the run's summary page.
            if std::env::var_os("GITHUB_ACTIONS").is_some() {
                eprintln!("::error title=Release plan::{error}");
            } else {
                eprintln!("xtask release-plan: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

fn gather(repo: &Path, args: &ReleasePlanArgs) -> Result<ReleaseInputs, String> {
    let (workspace_version, internal_deps) = from_metadata(&cargo_metadata(repo)?)?;
    let tags = git(repo, &["tag", "--list", "v*"])?;
    let existing_tags = String::from_utf8_lossy(&tags.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let sha_on_main = is_ancestor(repo, &args.sha, &args.main)?;
    let repository = args
        .repository
        .clone()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .unwrap_or_default();
    Ok(ReleaseInputs {
        event: args.event,
        git_ref: args.git_ref.clone(),
        workspace_version,
        internal_deps,
        existing_tags,
        sha_on_main,
        repository,
    })
}

/// `cargo metadata --no-deps` for the workspace at `repo`, parsed.
pub fn cargo_metadata(repo: &Path) -> Result<Value, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .current_dir(repo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|e| format!("running cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("parsing cargo metadata: {e}"))
}

fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|e| format!("running git {}: {e}", args.join(" ")))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// `git merge-base --is-ancestor`, which exits 0 for yes and 1 for no; anything else (an unknown ref) is an error.
fn is_ancestor(repo: &Path, sha: &str, main: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["merge-base", "--is-ancestor", sha, main])
        .output()
        .map_err(|e| format!("running git merge-base: {e}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git merge-base --is-ancestor {sha} {main} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// A push to `main` in the upstream repository, with Python's last release tagged.
    fn inputs(version: &str) -> ReleaseInputs {
        ReleaseInputs {
            event: Event::Push,
            git_ref: "refs/heads/main".to_owned(),
            workspace_version: v(version),
            internal_deps: Vec::new(),
            existing_tags: vec!["v0.6.5".to_owned(), "v0.6.6".to_owned()],
            sha_on_main: true,
            repository: UPSTREAM_REPOSITORY.to_owned(),
        }
    }

    fn tag_push(version: &str, tag: &str) -> ReleaseInputs {
        ReleaseInputs {
            git_ref: format!("refs/tags/{tag}"),
            // A rehearsal runs on a rust-main commit, which isn't on main.
            sha_on_main: false,
            ..inputs(version)
        }
    }

    fn pin(dependency: &str, requirement: &str) -> InternalDep {
        InternalDep {
            package: "lazy-github".to_owned(),
            dependency: dependency.to_owned(),
            requirement: requirement.to_owned(),
        }
    }

    #[test]
    fn a_version_bump_on_main_is_a_stable_release() {
        assert_eq!(
            plan(&inputs("1.0.0")),
            Ok(Plan::Release {
                tag: "v1.0.0".to_owned(),
                version: v("1.0.0"),
                prerelease: false,
                publish_crates: true,
            })
        );
    }

    #[test]
    fn an_unchanged_version_skips() {
        let mut inputs = inputs("1.0.0");
        inputs.existing_tags.push("v1.0.0".to_owned());
        assert!(matches!(plan(&inputs), Ok(Plan::Skip { .. })));
    }

    #[test]
    fn a_version_not_above_the_latest_release_errors() {
        assert_eq!(
            plan(&inputs("0.6.0")),
            Err(PlanError::NotNewer {
                version: v("0.6.0"),
                latest: v("0.6.6"),
            })
        );
    }

    #[test]
    fn prerelease_tags_dont_count_as_the_latest_release() {
        let mut inputs = inputs("1.0.0");
        inputs
            .existing_tags
            .extend(["v1.0.0-rc.1".to_owned(), "v2.0.0-rc.1".to_owned()]);
        assert!(matches!(plan(&inputs), Ok(Plan::Release { prerelease: false, .. })));
    }

    #[test]
    fn a_prerelease_tag_is_a_prerelease_without_crates() {
        assert_eq!(
            plan(&tag_push("0.1.0", "v0.1.0-rc.1")),
            Ok(Plan::Release {
                tag: "v0.1.0-rc.1".to_owned(),
                version: v("0.1.0"),
                prerelease: true,
                publish_crates: false,
            })
        );
    }

    #[test]
    fn a_prerelease_needs_no_version_above_the_latest_release() {
        // The pre-cutover rehearsal: 0.1.0 is below Python's 0.6.6.
        assert!(plan(&tag_push("0.1.0", "v0.1.0-rc.1")).is_ok());
    }

    #[test]
    fn a_stable_tag_push_errors() {
        assert_eq!(
            plan(&tag_push("1.0.0", "v1.0.0")),
            Err(PlanError::StableTagPush {
                tag: "v1.0.0".to_owned()
            })
        );
    }

    #[test]
    fn a_tag_for_another_version_errors() {
        for tag in [
            "v0.2.0-rc.1",
            "v0.1.00-rc.1",
            "v0.1.0rc1",
            "v0.1.0-",
            "v0.1.0-rc..1",
            "0.1.0-rc.1",
        ] {
            assert_eq!(
                plan(&tag_push("0.1.0", tag)),
                Err(PlanError::TagMismatch {
                    tag: tag.to_owned(),
                    version: v("0.1.0"),
                }),
                "{tag}"
            );
        }
    }

    #[test]
    fn a_stable_plan_from_a_commit_not_on_main_errors() {
        let mut push = inputs("1.0.0");
        push.sha_on_main = false;
        assert_eq!(plan(&push), Err(PlanError::NotOnMain { version: v("1.0.0") }));

        let dispatch = ReleaseInputs {
            event: Event::WorkflowDispatch,
            git_ref: "refs/heads/rust-main".to_owned(),
            ..push
        };
        assert_eq!(plan(&dispatch), Err(PlanError::NotOnMain { version: v("1.0.0") }));
    }

    #[test]
    fn a_dispatch_on_main_is_a_stable_release() {
        let dispatch = ReleaseInputs {
            event: Event::WorkflowDispatch,
            ..inputs("1.0.0")
        };
        assert!(matches!(plan(&dispatch), Ok(Plan::Release { prerelease: false, .. })));
    }

    #[test]
    fn a_push_to_another_branch_errors() {
        let push = ReleaseInputs {
            git_ref: "refs/heads/rust-main".to_owned(),
            ..inputs("1.0.0")
        };
        assert!(matches!(plan(&push), Err(PlanError::UnexpectedRef { .. })));
    }

    #[test]
    fn crates_are_published_only_upstream() {
        let fork = ReleaseInputs {
            repository: "someone/gh-lazy".to_owned(),
            ..inputs("1.0.0")
        };
        assert!(matches!(
            plan(&fork),
            Ok(Plan::Release {
                publish_crates: false,
                ..
            })
        ));
        let local = ReleaseInputs {
            repository: String::new(),
            ..inputs("1.0.0")
        };
        assert!(matches!(
            plan(&local),
            Ok(Plan::Release {
                publish_crates: false,
                ..
            })
        ));
    }

    #[test]
    fn mismatched_internal_pins_error() {
        let mut stale = inputs("1.0.0");
        stale.internal_deps = vec![pin("lazy-github-core", "=1.0.0"), pin("lazy-github-tui", "=0.1.0")];
        assert_eq!(
            plan(&stale),
            Err(PlanError::PinMismatch {
                package: "lazy-github".to_owned(),
                dependency: "lazy-github-tui".to_owned(),
                requirement: "=0.1.0".to_owned(),
                version: v("1.0.0"),
            })
        );
        // A caret requirement isn't a lockstep pin either.
        let mut caret = inputs("1.0.0");
        caret.internal_deps = vec![pin("lazy-github-core", "^1.0.0")];
        assert!(matches!(plan(&caret), Err(PlanError::PinMismatch { .. })));
        // The pins are checked on prerelease tags too, so a rehearsal catches them.
        let mut rehearsal = tag_push("1.0.0", "v1.0.0-rc.1");
        rehearsal.internal_deps = vec![pin("lazy-github-core", "=0.9.0")];
        assert!(matches!(plan(&rehearsal), Err(PlanError::PinMismatch { .. })));
    }

    #[test]
    fn matching_and_path_only_pins_pass() {
        let mut ok = inputs("1.0.0");
        ok.internal_deps = vec![pin("lazy-github-core", "=1.0.0"), pin("lazy-github-tui", "*")];
        assert!(matches!(plan(&ok), Ok(Plan::Release { .. })));
    }

    #[test]
    fn versions_parse_only_plain_semver() {
        assert_eq!(
            v("1.20.3"),
            Version {
                major: 1,
                minor: 20,
                patch: 3
            }
        );
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.2.3-rc.1",
            "1.2.x",
            "v1.2.3",
            "1..3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(v("0.10.0") > v("0.9.9"));
    }

    #[test]
    fn outputs_list_every_key() {
        let release = Plan::Release {
            tag: "v1.0.0-rc.1".to_owned(),
            version: v("1.0.0"),
            prerelease: true,
            publish_crates: false,
        };
        assert_eq!(
            release.outputs(),
            "release=true\nversion=1.0.0\ntag=v1.0.0-rc.1\nprerelease=true\npublish_crates=false\n"
        );
        let skip = Plan::Skip {
            reason: "no".to_owned(),
        };
        assert_eq!(
            skip.outputs(),
            "release=false\nversion=\ntag=\nprerelease=false\npublish_crates=false\n"
        );
    }

    #[test]
    fn metadata_gives_the_version_and_the_internal_dependencies() {
        let metadata = serde_json::json!({
            "packages": [
                {"name": "lazy-github", "version": "1.0.0", "dependencies": [
                    {"name": "lazy-github-core", "req": "=1.0.0", "kind": null},
                    {"name": "clap", "req": "^4.6", "kind": null},
                    {"name": "fake-gh", "req": "*", "kind": "dev"},
                ]},
                {"name": "lazy-github-core", "version": "1.0.0", "dependencies": []},
                {"name": "fake-gh", "version": "1.0.0", "dependencies": []},
            ]
        });
        let (version, deps) = from_metadata(&metadata).unwrap();
        assert_eq!(version, v("1.0.0"));
        assert_eq!(deps, [pin("lazy-github-core", "=1.0.0")]);
    }

    /// The real workspace: its pins pass the lockstep check `release-plan` runs.
    #[test]
    fn the_workspace_pins_match_its_version() {
        let metadata = cargo_metadata(&crate::parity::cli::repo_root()).unwrap();
        let (version, deps) = from_metadata(&metadata).unwrap();
        check_pins(version, &deps).unwrap();
    }

    /// The crates.io packaging (#204): exactly the three app crates are publishable, each with the metadata its
    /// crates.io page shows, and their dependencies on each other are exact pins of the workspace version.
    #[test]
    fn published_crates_have_their_metadata_and_exact_pins() {
        const PUBLISHED: [&str; 3] = ["lazy-github", "lazy-github-core", "lazy-github-tui"];
        let metadata = cargo_metadata(&crate::parity::cli::repo_root()).unwrap();
        let packages = metadata["packages"].as_array().unwrap();
        let mut publishable = Vec::new();
        for package in packages {
            let name = package["name"].as_str().unwrap();
            // `publish` is null for "anywhere", [] for `publish = false`, or the allowed registries.
            if package["publish"] == serde_json::json!([]) {
                continue;
            }
            assert_eq!(package["publish"], serde_json::json!(["crates-io"]), "{name}");
            publishable.push(name);
            for key in ["description", "license", "repository", "homepage", "readme"] {
                assert!(
                    package[key].as_str().is_some_and(|value| !value.is_empty()),
                    "{name} has no {key}"
                );
            }
            for key in ["keywords", "categories", "authors"] {
                assert!(
                    package[key].as_array().is_some_and(|list| !list.is_empty()),
                    "{name} has no {key}"
                );
            }
        }
        publishable.sort_unstable();
        assert_eq!(publishable, PUBLISHED, "fake-gh and xtask must stay unpublishable");

        let (version, deps) = from_metadata(&metadata).unwrap();
        let expected = format!("={version}");
        let published_deps: Vec<_> = deps
            .iter()
            .filter(|dep| PUBLISHED.contains(&dep.package.as_str()))
            .collect();
        // lazy-github on core and tui, and tui on core.
        assert_eq!(published_deps.len(), 3, "{published_deps:?}");
        for dep in published_deps {
            assert_eq!(dep.requirement, expected, "{} -> {}", dep.package, dep.dependency);
        }
    }
}
