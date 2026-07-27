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

    /// How the reference is shown in a column: a hash is cut to its short form, a tag
    /// name is worth reading in full.
    pub fn label(&self) -> String {
        match self {
            ImageRef::Commit(sha) => sha.chars().take(SHORT_HASH_LEN).collect(),
            ImageRef::Tag(name) => name.clone(),
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
    pub fn short(&self) -> String {
        self.commit.chars().take(SHORT_HASH_LEN).collect()
    }

    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("{} {}", name, self.short()),
            None => format!("HEAD {}", self.short()),
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

/// Why a repository has no target. Kept apart from a plain message because the two cases
/// are not failures of the same kind: a repository without tags is a fact about the
/// repository, a broken `ls-remote` is a fact about this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    NoTags,
    Git(String),
}

/// What is known about a repository's target commit.
#[derive(Debug, Clone)]
pub enum RemoteState {
    Loading,
    Ready(Target),
    /// The namespace has no entry in `repos.json`, so there is no repository to ask.
    Unconfigured,
    /// Tag mode against a repository that has not been tagged yet.
    NoTags,
    Failed(String),
}

/// Which of the five things a row can be. Ordered the way a reader should look at them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Behind,
    Failed(Failure),
    Resolving,
    Unknown(Unresolved),
    InSync,
}

/// Side that could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Kube,
    Git,
}

/// Why there is nothing to compare. Shown as a short code in its own column, so the five
/// meanings that used to share one shade of grey can be told apart at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unresolved {
    Sidecar,
    Digest,
    NoConfig,
    NoTags,
    WrongMode,
    Finished,
    Empty,
}

impl Verdict {
    /// Carries the same meaning as the colour, so a row still reads in a monochrome
    /// terminal and under colour blindness.
    pub fn glyph(self) -> char {
        match self {
            Verdict::InSync => '●',
            Verdict::Behind => '▼',
            Verdict::Failed(_) => '✗',
            Verdict::Resolving => '⠙',
            Verdict::Unknown(_) => '○',
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Verdict::Failed(Failure::Kube) => "k8s",
            Verdict::Failed(Failure::Git) => "git",
            Verdict::Unknown(Unresolved::Sidecar) => "sidecar",
            Verdict::Unknown(Unresolved::Digest) => "digest",
            Verdict::Unknown(Unresolved::NoConfig) => "no-cfg",
            Verdict::Unknown(Unresolved::NoTags) => "no-tags",
            Verdict::Unknown(Unresolved::WrongMode) => "mode",
            Verdict::Unknown(Unresolved::Finished) => "done",
            Verdict::Unknown(Unresolved::Empty) => "no-pods",
            Verdict::Behind | Verdict::Resolving | Verdict::InSync => "",
        }
    }

    /// Sort weight: what somebody opening the tool has to act on comes first.
    pub fn severity(self) -> u8 {
        match self {
            Verdict::Behind => 0,
            Verdict::Failed(_) => 1,
            Verdict::Resolving => 2,
            Verdict::Unknown(_) => 3,
            Verdict::InSync => 4,
        }
    }

    fn worse(self, other: Verdict) -> Verdict {
        match other.severity() < self.severity() {
            true => other,
            false => self,
        }
    }

    /// Verdict of a whole made of parts — the containers of a pod, the pods of a namespace.
    ///
    /// Anything lagging drags the whole down. Parts nobody can judge are left out as long as
    /// something else could be judged: a sidecar pinned to its own version says nothing
    /// about the release, and neither does somebody else's pod that happens to share the
    /// namespace. Only when nothing at all can be judged does the whole take on the reason
    /// why.
    pub fn of_parts(parts: impl IntoIterator<Item = Verdict>) -> Verdict {
        let mut judged: Option<Verdict> = None;
        let mut unjudged: Option<Verdict> = None;

        for verdict in parts {
            let slot = match verdict {
                Verdict::Unknown(_) => &mut unjudged,
                _ => &mut judged,
            };
            *slot = Some(match slot.take() {
                Some(current) => current.worse(verdict),
                None => verdict,
            });
        }

        judged
            .or(unjudged)
            .unwrap_or(Verdict::Unknown(Unresolved::Empty))
    }
}

/// Mode the lookup was made in, the namespace it belongs to, and the resolved target.
pub type LookupResult = (Mode, String, Result<Target, LookupError>);

pub fn resolve(mode: Mode, url: &str) -> Result<Target, LookupError> {
    match mode {
        Mode::Commit => latest_commit(url),
        Mode::Tag => latest_tag(url),
    }
}

/// Commit that the remote `HEAD` points at, i.e. the tip of the default branch.
pub fn latest_commit(url: &str) -> Result<Target, LookupError> {
    let stdout = ls_remote(&[url, "HEAD"])?;

    match stdout.split_whitespace().next() {
        Some(sha) => Ok(Target {
            commit: sha.to_string(),
            ..Target::default()
        }),
        None => Err(LookupError::Git("remote returned no HEAD".to_string())),
    }
}

/// Commit behind the highest version tag. Ordering is left to `git`, whose version sort
/// puts `v0.1.100` above `v0.1.99` — plain alphabetical would not.
pub fn latest_tag(url: &str) -> Result<Target, LookupError> {
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
pub fn judge(image_ref: Option<&ImageRef>, remote: Option<&RemoteState>) -> Verdict {
    let target = match remote {
        None | Some(RemoteState::Loading) => return Verdict::Resolving,
        Some(RemoteState::Unconfigured) => return Verdict::Unknown(Unresolved::NoConfig),
        Some(RemoteState::NoTags) => return Verdict::Unknown(Unresolved::NoTags),
        Some(RemoteState::Failed(_)) => return Verdict::Failed(Failure::Git),
        Some(RemoteState::Ready(target)) => target,
    };

    let Some(image_ref) = image_ref else {
        return Verdict::Unknown(Unresolved::Digest);
    };

    match image_ref {
        ImageRef::Commit(hash) => match target
            .commit
            .to_lowercase()
            .starts_with(&hash.to_lowercase())
        {
            true => Verdict::InSync,
            false => Verdict::Behind,
        },
        // by commit rather than by name: two tags can point at the same one, and then the
        // pod is running exactly what the reference asks for
        ImageRef::Tag(name) => match target.tags.get(name) {
            Some(commit) => match commit == &target.commit {
                true => Verdict::InSync,
                false => Verdict::Behind,
            },
            // without a tag list there is nothing to look the name up in, which says
            // something about the mode rather than about the image
            None if target.tags.is_empty() => Verdict::Unknown(Unresolved::WrongMode),
            None => Verdict::Unknown(Unresolved::Sidecar),
        },
    }
}

/// Shells out to `git` so that the user's existing credentials and helpers apply —
/// no API token needed.
fn ls_remote(args: &[&str]) -> Result<String, LookupError> {
    let output = match Command::new("git").arg("ls-remote").args(args).output() {
        Ok(x) => x,
        Err(e) => return Err(LookupError::Git(format!("cannot run git: {}", e))),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git ls-remote failed")
            .trim()
            .to_string();

        return Err(LookupError::Git(reason));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Expects `git ls-remote --tags --sort=-v:refname` output: the newest tag comes first,
/// and an annotated tag also has a `^{}` line carrying the commit it points at.
fn pick_latest_tag(stdout: &str) -> Result<Target, LookupError> {
    let refs: Vec<(&str, &str)> = stdout
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(sha, name)| (sha.trim(), name.trim()))
        .collect();

    let Some((_, top)) = refs.first() else {
        return Err(LookupError::NoTags);
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
        None => Err(LookupError::Git(
            "cannot resolve the latest tag".to_string(),
        )),
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
    fn commit_reference_is_labelled_by_its_short_hash() {
        let running = ImageRef::Commit("4f1c0f2c8b3d4e5a".to_string());

        assert_that!(running.label()).is_equal_to("4f1c0f2c".to_string());
    }

    #[test]
    fn tag_reference_is_labelled_in_full() {
        let release = "v10.2.400-rc1";

        asserting!("a version cut in half is not a version")
            .that(&ImageRef::Tag(release.to_string()).label())
            .is_equal_to(release.to_string());
    }

    #[test]
    fn commit_matching_the_target_is_in_sync() {
        let short = "bca81e5e";
        let running = ImageRef::Commit(short.to_string());

        assert_that!(judge(Some(&running), Some(&at_commit(&sha(short)))))
            .is_equal_to(Verdict::InSync);
    }

    #[test]
    fn commit_differing_from_the_target_is_behind() {
        let running = ImageRef::Commit("30f43732".to_string());

        assert_that!(judge(Some(&running), Some(&at_commit(&sha("4db5f21d")))))
            .is_equal_to(Verdict::Behind);
    }

    #[test]
    fn commit_is_compared_ignoring_case() {
        let short = "F0F1AAFC";
        let running = ImageRef::Commit(short.to_string());

        assert_that!(judge(
            Some(&running),
            Some(&at_commit(&sha(&short.to_lowercase())))
        ))
        .is_equal_to(Verdict::InSync);
    }

    #[test]
    fn newest_tag_is_in_sync() {
        let newest = "v0.2.3";
        let remote = released(&[
            (&sha("11111111"), &format!("refs/tags/{}", newest)),
            (&sha("22222222"), "refs/tags/v0.2.2"),
        ]);

        assert_that!(judge(
            Some(&ImageRef::Tag(newest.to_string())),
            Some(&remote)
        ))
        .is_equal_to(Verdict::InSync);
    }

    #[test]
    fn superseded_tag_is_behind() {
        let superseded = "v0.0.30";
        let remote = released(&[
            (&sha("33333333"), "refs/tags/v0.0.37"),
            (&sha("44444444"), &format!("refs/tags/{}", superseded)),
        ]);

        assert_that!(judge(
            Some(&ImageRef::Tag(superseded.to_string())),
            Some(&remote)
        ))
        .is_equal_to(Verdict::Behind);
    }

    #[test]
    fn tag_pointing_at_the_reference_commit_is_in_sync() {
        let shared = sha("aabbccdd");
        let remote = released(&[(&shared, "refs/tags/v2.1.0"), (&shared, "refs/tags/v2.0.9")]);

        asserting!("an older tag name on the very commit the reference names is not behind")
            .that(&judge(
                Some(&ImageRef::Tag("v2.0.9".to_string())),
                Some(&remote),
            ))
            .is_equal_to(Verdict::InSync);
    }

    #[test]
    fn version_absent_from_the_repository_is_a_sidecar() {
        let remote = released(&[(&sha("55555555"), "refs/tags/v0.1.1")]);
        let sidecar = ImageRef::Tag("1.26.2".to_string());

        asserting!("a sidecar version must not be judged against the application's releases")
            .that(&judge(Some(&sidecar), Some(&remote)))
            .is_equal_to(Verdict::Unknown(Unresolved::Sidecar));
    }

    #[test]
    fn release_in_commit_mode_points_at_the_other_mode() {
        let running = ImageRef::Tag("v0.1.20".to_string());

        asserting!("an application running a release is not a foreign image")
            .that(&judge(Some(&running), Some(&at_commit(&sha("fa6e1e57")))))
            .is_equal_to(Verdict::Unknown(Unresolved::WrongMode));
    }

    #[test]
    fn container_without_a_reference_is_a_digest() {
        assert_that!(judge(None, Some(&at_commit(&sha("ff8ef895")))))
            .is_equal_to(Verdict::Unknown(Unresolved::Digest));
    }

    #[test]
    fn namespace_missing_from_the_config_is_unconfigured() {
        let running = ImageRef::Commit("c3cc3026".to_string());

        assert_that!(judge(Some(&running), Some(&RemoteState::Unconfigured)))
            .is_equal_to(Verdict::Unknown(Unresolved::NoConfig));
    }

    #[test]
    fn repository_without_tags_is_reported_as_such() {
        let running = ImageRef::Tag("v3.1.0".to_string());

        assert_that!(judge(Some(&running), Some(&RemoteState::NoTags)))
            .is_equal_to(Verdict::Unknown(Unresolved::NoTags));
    }

    #[test]
    fn lookup_not_started_yet_is_resolving() {
        let running = ImageRef::Commit("2a4fd3af".to_string());

        assert_that!(judge(Some(&running), None)).is_equal_to(Verdict::Resolving);
    }

    #[test]
    fn pending_lookup_is_resolving() {
        let running = ImageRef::Commit("7bd51e08".to_string());

        assert_that!(judge(Some(&running), Some(&RemoteState::Loading)))
            .is_equal_to(Verdict::Resolving);
    }

    #[test]
    fn broken_lookup_is_a_git_failure() {
        let running = ImageRef::Commit("f9860c4b".to_string());
        let failure = RemoteState::Failed("repository not found".to_string());

        assert_that!(judge(Some(&running), Some(&failure)))
            .is_equal_to(Verdict::Failed(Failure::Git));
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
    fn repository_without_tags_reports_that_it_has_none() {
        assert_that!(pick_latest_tag("")).is_err_containing(LookupError::NoTags);
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
    fn one_lagging_part_drags_the_whole_down() {
        let verdicts = [Verdict::InSync, Verdict::Behind, Verdict::InSync];

        asserting!("an up to date part must not vouch for a lagging one next to it")
            .that(&Verdict::of_parts(verdicts))
            .is_equal_to(Verdict::Behind);
    }

    #[test]
    fn whole_made_of_up_to_date_parts_is_in_sync() {
        let verdicts = [Verdict::InSync, Verdict::InSync];

        assert_that!(Verdict::of_parts(verdicts)).is_equal_to(Verdict::InSync);
    }

    #[test]
    fn part_nobody_can_judge_is_left_out() {
        let verdicts = [Verdict::InSync, Verdict::Unknown(Unresolved::Sidecar)];

        asserting!("a sidecar, or somebody else's pod in the namespace, says nothing")
            .that(&Verdict::of_parts(verdicts))
            .is_equal_to(Verdict::InSync);
    }

    #[test]
    fn lagging_part_outweighs_a_broken_lookup() {
        let verdicts = [Verdict::Failed(Failure::Git), Verdict::Behind];

        asserting!("something to fix outranks something that could not be checked")
            .that(&Verdict::of_parts(verdicts))
            .is_equal_to(Verdict::Behind);
    }

    #[test]
    fn whole_with_nothing_judgeable_takes_on_the_reason() {
        let verdicts = [
            Verdict::Unknown(Unresolved::Digest),
            Verdict::Unknown(Unresolved::Digest),
        ];

        assert_that!(Verdict::of_parts(verdicts)).is_equal_to(Verdict::Unknown(Unresolved::Digest));
    }

    #[test]
    fn whole_without_parts_has_nothing_to_compare() {
        assert_that!(Verdict::of_parts([])).is_equal_to(Verdict::Unknown(Unresolved::Empty));
    }

    #[test]
    fn lagging_row_sorts_above_an_up_to_date_one() {
        asserting!("the table puts what needs acting on first")
            .that(&Verdict::Behind.severity())
            .is_less_than(Verdict::InSync.severity());
    }

    #[test]
    fn each_state_has_its_own_glyph() {
        let glyphs = [
            Verdict::Behind,
            Verdict::Failed(Failure::Git),
            Verdict::Resolving,
            Verdict::Unknown(Unresolved::Digest),
            Verdict::InSync,
        ]
        .map(Verdict::glyph);

        let mut unique = glyphs.to_vec();
        unique.sort_unstable();
        unique.dedup();

        asserting!("a glyph shared by two states would carry no information")
            .that(&unique.len())
            .is_equal_to(glyphs.len());
    }

    #[test]
    fn every_reason_for_grey_has_its_own_code() {
        let codes = [
            Unresolved::Sidecar,
            Unresolved::Digest,
            Unresolved::NoConfig,
            Unresolved::NoTags,
            Unresolved::WrongMode,
            Unresolved::Finished,
            Unresolved::Empty,
        ]
        .map(|reason| Verdict::Unknown(reason).code());

        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();

        assert_that!(unique.len()).is_equal_to(codes.len());
    }
}
