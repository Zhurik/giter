use crate::errors::MsgError;
use clap::ValueEnum;
use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::thread;

const REFS_TAGS: &str = "refs/tags/";
/// Suffix `git` puts on the ref that an annotated tag points at.
const PEELED: &str = "^{}";
const SHORT_HASH_LEN: usize = 8;
const MIN_HASH_LEN: usize = 7;
const MAX_HASH_LEN: usize = 40;

/// What CI baked into an image tag. Branch builds are tagged `<short sha>-<pipeline id>`,
/// releases `<git tag>-<pipeline id>`, so the same field means different things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageRef {
    Commit(String),
    Tag(String),
}

impl ImageRef {
    pub fn from_image_tag(tag: &str) -> Option<ImageRef> {
        let reference = strip_pipeline_id(tag);
        if reference.is_empty() {
            return None;
        }

        let looks_like_hash = (MIN_HASH_LEN..=MAX_HASH_LEN).contains(&reference.len())
            && reference.chars().all(|c| c.is_ascii_hexdigit());

        Some(match looks_like_hash {
            true => ImageRef::Commit(reference.to_string()),
            false => ImageRef::Tag(reference.to_string()),
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            ImageRef::Commit(x) | ImageRef::Tag(x) => x,
        }
    }
}

/// CI appends `-<pipeline id>`; what precedes it is the commit or the tag it built from.
/// Only the trailing group is dropped, so `v1.0.0-rc1-123` keeps its `-rc1`.
fn strip_pipeline_id(tag: &str) -> &str {
    match tag.rsplit_once('-') {
        Some((head, last))
            if !head.is_empty() && !last.is_empty() && last.chars().all(|c| c.is_ascii_digit()) =>
        {
            head
        }
        _ => tag,
    }
}

/// What a running pod is expected to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, ValueEnum)]
pub enum Mode {
    /// Tip of the default branch
    #[default]
    Commit,
    /// Commit the latest tag points at — what production is supposed to run
    Tag,
}

impl Mode {
    pub fn toggled(self) -> Mode {
        match self {
            Mode::Commit => Mode::Tag,
            Mode::Tag => Mode::Commit,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Commit => "commit",
            Mode::Tag => "tag",
        }
    }
}

/// Commit the pods are compared against, plus the tag name when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    pub commit: String,
    pub name: Option<String>,
    /// Every tag of the repository mapped to its commit. Only filled in tag mode, where it
    /// tells this repository's tags apart from a sidecar's version and resolves the commit
    /// of whatever tag a pod actually runs.
    pub tags: HashMap<String, String>,
}

impl Target {
    pub fn label(&self) -> String {
        let short: String = self.commit.chars().take(SHORT_HASH_LEN).collect();

        match &self.name {
            Some(name) => format!("{} {}", name, short),
            None => format!("HEAD {}", short),
        }
    }

    /// Commit an image reference stands for, as far as this target can tell.
    pub fn commit_of(&self, image_ref: &ImageRef) -> Option<String> {
        match image_ref {
            ImageRef::Commit(sha) => Some(sha.clone()),
            ImageRef::Tag(name) => self.tags.get(name).cloned(),
        }
    }
}

/// What is known about a repository's target commit.
#[derive(Debug, Clone)]
pub enum RemoteState {
    Loading,
    Ready(Target),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStatus {
    Latest,
    Outdated,
    Unknown,
}

impl CommitStatus {
    /// Verdict for a group of pods, such as a whole namespace: one lagging pod makes the
    /// group lag, and the group is up to date only when every pod is known to be. A pod
    /// nobody can judge leaves the group's verdict open rather than vouching for it.
    pub fn combined(statuses: impl IntoIterator<Item = CommitStatus>) -> CommitStatus {
        let mut verdict = CommitStatus::Latest;
        let mut seen = false;

        for status in statuses {
            seen = true;

            match status {
                CommitStatus::Outdated => return CommitStatus::Outdated,
                CommitStatus::Unknown => verdict = CommitStatus::Unknown,
                CommitStatus::Latest => (),
            }
        }

        match seen {
            true => verdict,
            false => CommitStatus::Unknown,
        }
    }

    /// Verdict for a single pod: any lagging container drags the pod down, while containers
    /// nobody can judge — sidecars pinned to their own version — are ignored.
    pub fn worst_judged(statuses: impl IntoIterator<Item = CommitStatus>) -> CommitStatus {
        statuses
            .into_iter()
            .fold(CommitStatus::Unknown, |worst, status| {
                match (worst, status) {
                    (CommitStatus::Outdated, _) | (_, CommitStatus::Outdated) => {
                        CommitStatus::Outdated
                    }
                    (CommitStatus::Latest, _) | (_, CommitStatus::Latest) => CommitStatus::Latest,
                    _ => CommitStatus::Unknown,
                }
            })
    }
}

/// Mode the lookup was made in, the namespace it belongs to, and the resolved target.
pub type LookupResult = (Mode, String, Result<Target, MsgError>);

pub fn resolve(mode: Mode, url: &str) -> Result<Target, MsgError> {
    match mode {
        Mode::Commit => latest_commit(url),
        Mode::Tag => latest_tag(url),
    }
}

/// Commit that the remote `HEAD` points at, i.e. the tip of the default branch.
pub fn latest_commit(url: &str) -> Result<Target, MsgError> {
    let stdout = ls_remote(&[url, "HEAD"])?;

    match stdout.split_whitespace().next() {
        Some(sha) => Ok(Target {
            commit: sha.to_string(),
            ..Target::default()
        }),
        None => Err(MsgError::new("remote returned no HEAD")),
    }
}

/// Commit behind the highest version tag. Ordering is left to `git`, whose version sort
/// puts `v0.1.100` above `v0.1.99` — plain alphabetical would not.
pub fn latest_tag(url: &str) -> Result<Target, MsgError> {
    let stdout = ls_remote(&["--tags", "--sort=-v:refname", url])?;

    pick_latest_tag(&stdout)
}

/// Resolves one repository off-thread; the result arrives through `tx`.
pub fn spawn_lookup(tx: Sender<LookupResult>, mode: Mode, namespace: String, url: String) {
    thread::spawn(move || {
        let result = resolve(mode, &url);
        let _ = tx.send((mode, namespace, result));
    });
}

/// Whether a running image is what the target says it should be.
///
/// A commit is compared by hash. A tag is compared by name, and only when the repository
/// actually has a tag with that name — otherwise it belongs to something else, such as a
/// sidecar pinned to its own version, and there is nothing to judge.
pub fn commit_status(image_ref: Option<&ImageRef>, remote: Option<&RemoteState>) -> CommitStatus {
    let (image_ref, target) = match (image_ref, remote) {
        (Some(image_ref), Some(RemoteState::Ready(target))) if !image_ref.as_str().is_empty() => {
            (image_ref, target)
        }
        _ => return CommitStatus::Unknown,
    };

    match image_ref {
        ImageRef::Commit(hash) => {
            match target
                .commit
                .to_lowercase()
                .starts_with(&hash.to_lowercase())
            {
                true => CommitStatus::Latest,
                false => CommitStatus::Outdated,
            }
        }
        ImageRef::Tag(name) => match (&target.name, target.tags.contains_key(name)) {
            (Some(latest), true) => match latest == name {
                true => CommitStatus::Latest,
                false => CommitStatus::Outdated,
            },
            _ => CommitStatus::Unknown,
        },
    }
}

/// Shells out to `git` so that the user's existing credentials and helpers apply —
/// no API token needed.
fn ls_remote(args: &[&str]) -> Result<String, MsgError> {
    let output = match Command::new("git").arg("ls-remote").args(args).output() {
        Ok(x) => x,
        Err(e) => return Err(MsgError::new(&format!("cannot run git: {}", e))),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git ls-remote failed")
            .trim()
            .to_string();

        return Err(MsgError::new(&reason));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Expects `git ls-remote --tags --sort=-v:refname` output: the newest tag comes first,
/// and an annotated tag also has a `^{}` line carrying the commit it points at.
fn pick_latest_tag(stdout: &str) -> Result<Target, MsgError> {
    let refs: Vec<(&str, &str)> = stdout
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(sha, name)| (sha.trim(), name.trim()))
        .collect();

    let Some((_, top)) = refs.first() else {
        return Err(MsgError::new("no tags"));
    };

    let mut tags: HashMap<String, String> = HashMap::new();

    for (sha, reference) in &refs {
        let Some(name) = reference.strip_prefix(REFS_TAGS) else {
            continue;
        };

        // a peeled entry names the commit, so it wins over the tag object's own sha
        match name.strip_suffix(PEELED) {
            Some(peeled) => {
                tags.insert(peeled.to_string(), sha.to_string());
            }
            None => {
                tags.entry(name.to_string())
                    .or_insert_with(|| sha.to_string());
            }
        }
    }

    let name = top.strip_prefix(REFS_TAGS).unwrap_or(top);
    let name = name.strip_suffix(PEELED).unwrap_or(name);

    match tags.get(name) {
        Some(commit) => Ok(Target {
            commit: commit.clone(),
            name: Some(name.to_string()),
            tags,
        }),
        None => Err(MsgError::new("cannot resolve the latest tag")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speculoos::prelude::*;

    fn sha(short: &str) -> String {
        format!("{:a<40}", short)
    }

    fn ls_remote_output(refs: &[(&str, &str)]) -> String {
        refs.iter()
            .map(|(commit, reference)| format!("{}\t{}\n", commit, reference))
            .collect()
    }

    fn released(refs: &[(&str, &str)]) -> RemoteState {
        RemoteState::Ready(pick_latest_tag(&ls_remote_output(refs)).unwrap())
    }

    fn at_commit(commit: &str) -> RemoteState {
        RemoteState::Ready(Target {
            commit: commit.to_string(),
            ..Target::default()
        })
    }

    #[test]
    fn branch_build_is_read_as_a_commit() {
        let commit = "df811319";

        assert_that!(ImageRef::from_image_tag(&format!("{}-5432284", commit)))
            .is_equal_to(Some(ImageRef::Commit(commit.to_string())));
    }

    #[test]
    fn release_build_is_read_as_a_tag() {
        let release = "v0.1.3";

        assert_that!(ImageRef::from_image_tag(&format!("{}-5328619", release)))
            .is_equal_to(Some(ImageRef::Tag(release.to_string())));
    }

    #[test]
    fn prerelease_suffix_survives_the_pipeline_id() {
        let release = "v2.0.0-rc4";

        asserting!("only the trailing numeric group is the pipeline id")
            .that(&ImageRef::from_image_tag(&format!("{}-5041301", release)))
            .is_equal_to(Some(ImageRef::Tag(release.to_string())));
    }

    #[test]
    fn version_without_a_pipeline_id_is_read_as_a_tag() {
        let version = "1.30.0";

        assert_that!(ImageRef::from_image_tag(version))
            .is_equal_to(Some(ImageRef::Tag(version.to_string())));
    }

    #[test]
    fn commit_matching_the_target_is_latest() {
        let short = "bca81e5e";
        let running = ImageRef::Commit(short.to_string());

        assert_that!(commit_status(Some(&running), Some(&at_commit(&sha(short)))))
            .is_equal_to(CommitStatus::Latest);
    }

    #[test]
    fn commit_differing_from_the_target_is_outdated() {
        let running = ImageRef::Commit("30f43732".to_string());

        assert_that!(commit_status(
            Some(&running),
            Some(&at_commit(&sha("4db5f21d")))
        ))
        .is_equal_to(CommitStatus::Outdated);
    }

    #[test]
    fn commit_is_compared_ignoring_case() {
        let short = "F0F1AAFC";
        let running = ImageRef::Commit(short.to_string());

        assert_that!(commit_status(
            Some(&running),
            Some(&at_commit(&sha(&short.to_lowercase())))
        ))
        .is_equal_to(CommitStatus::Latest);
    }

    #[test]
    fn newest_tag_is_latest() {
        let newest = "v0.2.3";
        let remote = released(&[
            (&sha("11111111"), &format!("refs/tags/{}", newest)),
            (&sha("22222222"), "refs/tags/v0.2.2"),
        ]);

        assert_that!(commit_status(
            Some(&ImageRef::Tag(newest.to_string())),
            Some(&remote)
        ))
        .is_equal_to(CommitStatus::Latest);
    }

    #[test]
    fn superseded_tag_is_outdated() {
        let superseded = "v0.0.30";
        let remote = released(&[
            (&sha("33333333"), "refs/tags/v0.0.37"),
            (&sha("44444444"), &format!("refs/tags/{}", superseded)),
        ]);

        assert_that!(commit_status(
            Some(&ImageRef::Tag(superseded.to_string())),
            Some(&remote)
        ))
        .is_equal_to(CommitStatus::Outdated);
    }

    #[test]
    fn version_absent_from_the_repository_is_unknown() {
        let remote = released(&[(&sha("55555555"), "refs/tags/v0.1.1")]);
        let sidecar = ImageRef::Tag("1.26.2".to_string());

        asserting!("a sidecar version must not be judged against the application's releases")
            .that(&commit_status(Some(&sidecar), Some(&remote)))
            .is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn release_is_unknown_until_tags_are_fetched() {
        let running = ImageRef::Tag("v0.1.20".to_string());

        asserting!("commit mode has no tag list to resolve a release against")
            .that(&commit_status(
                Some(&running),
                Some(&at_commit(&sha("fa6e1e57"))),
            ))
            .is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn container_without_a_reference_is_unknown() {
        assert_that!(commit_status(None, Some(&at_commit(&sha("ff8ef895")))))
            .is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn missing_lookup_is_unknown() {
        let running = ImageRef::Commit("c3cc3026".to_string());

        assert_that!(commit_status(Some(&running), None)).is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn pending_lookup_is_unknown() {
        let running = ImageRef::Commit("2a4fd3af".to_string());

        assert_that!(commit_status(Some(&running), Some(&RemoteState::Loading)))
            .is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn failed_lookup_is_unknown() {
        let running = ImageRef::Commit("f9860c4b".to_string());
        let failure = RemoteState::Failed("repository not found".to_string());

        assert_that!(commit_status(Some(&running), Some(&failure)))
            .is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn newest_release_is_named_after_its_tag() {
        let newest = "v1.1.1";
        let target = pick_latest_tag(&ls_remote_output(&[
            (&sha("66666666"), &format!("refs/tags/{}", newest)),
            (&sha("77777777"), "refs/tags/v1.0.9"),
        ]))
        .unwrap();

        assert_that!(target.name)
            .is_some()
            .is_equal_to(newest.to_string());
    }

    #[test]
    fn annotated_tag_resolves_to_the_commit_it_points_at() {
        let pointed_at = sha("347a96bc");
        let target = pick_latest_tag(&ls_remote_output(&[
            (&pointed_at, "refs/tags/v0.2.2^{}"),
            (&sha("0f835c2a"), "refs/tags/v0.2.2"),
        ]))
        .unwrap();

        asserting!("the peeled ref names the commit, the other one names the tag object")
            .that(&target.commit)
            .is_equal_to(pointed_at);
    }

    #[test]
    fn lightweight_tag_resolves_to_its_own_sha() {
        let newest = sha("65858222");
        let target = pick_latest_tag(&ls_remote_output(&[
            (&newest, "refs/tags/v0.2.0"),
            (&sha("7d96e0d5"), "refs/tags/v0.1.100^{}"),
        ]))
        .unwrap();

        assert_that!(target.commit).is_equal_to(newest);
    }

    #[test]
    fn superseded_tag_resolves_to_its_own_commit() {
        let superseded_at = sha("b5873ad6");
        let target = pick_latest_tag(&ls_remote_output(&[
            (&sha("fb262f28"), "refs/tags/v0.2.1"),
            (&superseded_at, "refs/tags/v0.2.0"),
        ]))
        .unwrap();

        assert_that!(target.commit_of(&ImageRef::Tag("v0.2.0".to_string())))
            .is_some()
            .is_equal_to(superseded_at);
    }

    #[test]
    fn tag_the_repository_does_not_have_resolves_to_nothing() {
        let target = pick_latest_tag(&ls_remote_output(&[(
            &sha("88888888"),
            "refs/tags/v0.1.16",
        )]))
        .unwrap();

        assert_that!(target.commit_of(&ImageRef::Tag("v9.9.9".to_string()))).is_none();
    }

    #[test]
    fn commit_reference_resolves_to_itself() {
        let short = "99f6dabd";
        let target =
            pick_latest_tag(&ls_remote_output(&[(&sha("aaaaaaa1"), "refs/tags/v0.1.7")])).unwrap();

        assert_that!(target.commit_of(&ImageRef::Commit(short.to_string())))
            .is_some()
            .is_equal_to(short.to_string());
    }

    #[test]
    fn repository_without_tags_is_an_error() {
        assert_that!(pick_latest_tag("")).is_err();
    }

    #[test]
    fn tagged_target_is_labelled_with_its_tag() {
        let release = "v0.1.33";
        let short = "bd812acb";
        let target = Target {
            commit: sha(short),
            name: Some(release.to_string()),
            ..Target::default()
        };

        assert_that!(target.label()).is_equal_to(format!("{} {}", release, short));
    }

    #[test]
    fn untagged_target_is_labelled_as_head() {
        let short = "15114a79";
        let target = Target {
            commit: sha(short),
            ..Target::default()
        };

        assert_that!(target.label()).is_equal_to(format!("HEAD {}", short));
    }

    #[test]
    fn group_with_one_lagging_member_is_outdated() {
        let statuses = [
            CommitStatus::Latest,
            CommitStatus::Outdated,
            CommitStatus::Latest,
        ];

        assert_that!(CommitStatus::combined(statuses)).is_equal_to(CommitStatus::Outdated);
    }

    #[test]
    fn group_where_every_member_is_up_to_date_is_latest() {
        let statuses = [CommitStatus::Latest, CommitStatus::Latest];

        assert_that!(CommitStatus::combined(statuses)).is_equal_to(CommitStatus::Latest);
    }

    #[test]
    fn group_holding_an_unjudged_member_is_unknown() {
        let statuses = [CommitStatus::Latest, CommitStatus::Unknown];

        asserting!("an unjudged member leaves the group's verdict open")
            .that(&CommitStatus::combined(statuses))
            .is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn lagging_member_outweighs_an_unjudged_one() {
        let statuses = [CommitStatus::Unknown, CommitStatus::Outdated];

        assert_that!(CommitStatus::combined(statuses)).is_equal_to(CommitStatus::Outdated);
    }

    #[test]
    fn empty_group_is_unknown() {
        assert_that!(CommitStatus::combined([])).is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn lagging_container_drags_the_pod_down() {
        let statuses = [CommitStatus::Latest, CommitStatus::Outdated];

        asserting!("an up to date container must not vouch for a lagging one next to it")
            .that(&CommitStatus::worst_judged(statuses))
            .is_equal_to(CommitStatus::Outdated);
    }

    #[test]
    fn unjudged_container_does_not_hold_the_pod_back() {
        let statuses = [CommitStatus::Unknown, CommitStatus::Latest];

        asserting!("a sidecar pinned to its own version says nothing about the release")
            .that(&CommitStatus::worst_judged(statuses))
            .is_equal_to(CommitStatus::Latest);
    }

    #[test]
    fn pod_with_nothing_judgeable_is_unknown() {
        let statuses = [CommitStatus::Unknown, CommitStatus::Unknown];

        assert_that!(CommitStatus::worst_judged(statuses)).is_equal_to(CommitStatus::Unknown);
    }

    #[test]
    fn pod_without_containers_is_unknown() {
        assert_that!(CommitStatus::worst_judged([])).is_equal_to(CommitStatus::Unknown);
    }
}
