use crate::caller::browser::{commit_url, open_with_hash};
use crate::errors::MsgError;
use crate::git::remote::{
    judge, spawn_lookup, Failure, ImageRef, LookupError, LookupResult, Mode, RemoteState,
    Unresolved, Verdict,
};
use crate::k8s::context::Cluster;
use crate::k8s::pods::{MyContainer, MyPod};
use crate::storage::json_storage::JsonStorage;
use crate::text::{family_prefix, shared_prefix, short_duration, split_pod_name};
use k8s_openapi::chrono::Utc;
use kube::Client;
use ratatui::widgets::TableState;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

/// How long a reported action stays in the footer before the summary comes back.
const NOTE_TTL: Duration = Duration::from_secs(6);
/// Deployed commits shown per namespace before the rest is counted as `+N`.
const DEPLOYED_GROUPS: usize = 2;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// Frames one spinner step lasts, at roughly ten frames per second.
const SPINNER_STEP: usize = 2;
const MISSING: &str = "—";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Detail,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Normal,
    Search,
    Help,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Namespaces,
    Pods,
    Containers,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Problems,
    Unknown,
}

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Problems => "problems",
            Filter::Unknown => "unknown",
        }
    }

    fn next(self) -> Filter {
        match self {
            Filter::All => Filter::Problems,
            Filter::Problems => Filter::Unknown,
            Filter::Unknown => Filter::All,
        }
    }

    fn accepts(self, verdict: Verdict) -> bool {
        match self {
            Filter::All => true,
            Filter::Problems => matches!(verdict, Verdict::Behind | Verdict::Failed(_)),
            Filter::Unknown => matches!(verdict, Verdict::Resolving | Verdict::Unknown(_)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Status,
    Name,
    Pods,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Sort::Status => "status",
            Sort::Name => "name",
            Sort::Pods => "pods",
        }
    }

    fn next(self) -> Sort {
        match self {
            Sort::Status => Sort::Name,
            Sort::Name => Sort::Pods,
            Sort::Pods => Sort::Status,
        }
    }
}

/// What is known about the pods of one namespace.
pub enum PodsState {
    Listing,
    Ready(Vec<MyPod>),
    Failed(String),
}

pub struct Entry {
    pub name: String,
    pub pods: PodsState,
}

impl Entry {
    fn pod_list(&self) -> &[MyPod] {
        match &self.pods {
            PodsState::Ready(pods) => pods,
            _ => &[],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Ok,
    Warn,
    Error,
}

/// One line about what just happened, or about why the selected namespace cannot be judged.
pub struct Note {
    pub kind: NoteKind,
    pub text: String,
    pub retry: bool,
    /// A standing problem, read off the state rather than reported once. Esc cannot make it
    /// go away, so the footer must not offer to.
    pub standing: bool,
}

#[derive(Default)]
pub struct Counts {
    pub in_sync: usize,
    pub behind: usize,
    pub failed: usize,
    pub unknown: usize,
}

/// What the references on screen are. The mode says what pods are compared *against*, not
/// what they carry: a cluster built from branches runs commits even in tag mode, and one
/// built from releases runs tags even in commit mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Commits,
    Tags,
    Mixed,
}

/// One deployed reference and how many pods run it.
pub struct Deployed {
    pub label: String,
    pub count: usize,
    pub verdict: Verdict,
}

/// A namespace as the tables show it.
pub struct NsRow {
    pub name: String,
    /// Characters of the shared prefix, dimmed instead of cut.
    pub prefix: usize,
    pub verdict: Verdict,
    pub pods: Option<usize>,
    pub deployed: Vec<Deployed>,
    pub hidden_deployed: usize,
    pub reference: String,
    pub note: String,
}

/// The namespace list of the detail screen, where families get a heading.
pub enum MiniRow {
    Family(String),
    Namespace(NsRow),
}

pub struct PodRow {
    pub prefix: String,
    pub replicaset: String,
    pub suffix: String,
    pub verdict: Verdict,
    pub reference: String,
    pub status: String,
    pub restarts: i32,
    pub age: String,
    pub readiness: String,
}

pub struct ContainerRow {
    pub name: String,
    pub tag: String,
    pub verdict: Verdict,
    /// Plain words for the verdict, so a grey row never stays unexplained.
    pub why: String,
}

/// Round the listing belongs to, the namespace, and what came back.
type PodsResult = (u64, String, Result<Vec<MyPod>, MsgError>);

pub struct App {
    pub cluster: Cluster,
    entries: Vec<Entry>,
    repos: JsonStorage,
    kube: Client,
    mode: Mode,
    /// Results are kept per mode, so switching back and forth costs nothing.
    remotes: HashMap<Mode, HashMap<String, RemoteState>>,
    lookups_tx: Sender<LookupResult>,
    lookups_rx: Receiver<LookupResult>,
    pods_tx: Sender<PodsResult>,
    pods_rx: Receiver<PodsResult>,

    screen: Screen,
    input: Input,
    pane: Pane,
    filter: Filter,
    sort: Sort,
    query: String,

    /// Selection is remembered by name: the table reorders as answers arrive, and the
    /// cursor has to stay on the row the user was looking at.
    ns_pick: Option<String>,
    pod_pick: Option<String>,
    cont_pick: Option<String>,
    ns_state: TableState,
    pod_state: TableState,
    cont_state: TableState,

    note: Option<(Note, Instant)>,
    /// Which round of pod listing is current: answers from an earlier one are dropped
    /// rather than allowed to reinstate a stale list of pods.
    listing_round: u64,
    listed_at: Option<Instant>,
    resolved_at: Option<Instant>,
    frame: usize,
    /// Set by the renderer: some keys mean something else when a pane is not on screen.
    narrow: bool,
    should_quit: bool,
}

impl App {
    pub fn new(
        names: Vec<String>,
        repos: JsonStorage,
        mode: Mode,
        cluster: Cluster,
        kube: Client,
    ) -> App {
        let (lookups_tx, lookups_rx) = channel();
        let (pods_tx, pods_rx) = channel();

        let mut app = App {
            cluster,
            entries: names
                .into_iter()
                .map(|name| Entry {
                    name,
                    pods: PodsState::Listing,
                })
                .collect(),
            repos,
            kube,
            mode,
            remotes: HashMap::new(),
            lookups_tx,
            lookups_rx,
            pods_tx,
            pods_rx,
            screen: Screen::Overview,
            input: Input::Normal,
            pane: Pane::Pods,
            filter: Filter::All,
            sort: Sort::Status,
            query: String::new(),
            ns_pick: None,
            pod_pick: None,
            cont_pick: None,
            ns_state: TableState::default(),
            pod_state: TableState::default(),
            cont_state: TableState::default(),
            note: None,
            listing_round: 0,
            listed_at: None,
            resolved_at: None,
            frame: 0,
            narrow: false,
            should_quit: false,
        };

        app.relist_pods();
        app.request_missing();
        app.normalize();

        app
    }

    // ---------------------------------------------------------------- state of the world

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn input(&self) -> Input {
        self.input
    }

    pub fn pane(&self) -> Pane {
        self.pane
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn filter(&self) -> Filter {
        self.filter
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn narrow(&self) -> bool {
        self.narrow
    }

    pub fn set_narrow(&mut self, narrow: bool) {
        self.narrow = narrow;
    }

    pub fn spinner(&self) -> char {
        SPINNER[(self.frame / SPINNER_STEP) % SPINNER.len()]
    }

    /// Picks up whatever the background work has finished, then keeps the selection
    /// pointing at rows that still exist.
    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.drain_pods();
        self.drain_lookups();
        self.normalize();
    }

    /// How many namespaces have been listed out of how many, while any is still pending.
    pub fn listing(&self) -> Option<(usize, usize)> {
        let done = self
            .entries
            .iter()
            .filter(|e| !matches!(e.pods, PodsState::Listing))
            .count();

        match done == self.entries.len() {
            true => None,
            false => Some((done, self.entries.len())),
        }
    }

    /// Repositories whose reference is still being fetched in the current mode.
    pub fn resolving(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(self.remote(&e.name), None | Some(RemoteState::Loading)))
            .count()
    }

    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();

        for entry in &self.entries {
            match self.namespace_verdict(entry) {
                Verdict::InSync => counts.in_sync += 1,
                Verdict::Behind => counts.behind += 1,
                Verdict::Failed(_) => counts.failed += 1,
                Verdict::Resolving | Verdict::Unknown(_) => counts.unknown += 1,
            }
        }

        counts
    }

    pub fn total_namespaces(&self) -> usize {
        self.entries.len()
    }

    pub fn total_pods(&self) -> usize {
        self.entries.iter().map(|e| e.pod_list().len()).sum()
    }

    pub fn total_containers(&self) -> usize {
        self.entries
            .iter()
            .flat_map(|e| e.pod_list())
            .map(|p| p.containers.len())
            .sum()
    }

    /// How long ago each side last answered, e.g. `k8s 4m ago · git 12s ago`.
    pub fn freshness(&self) -> String {
        let parts: Vec<String> = [("k8s", self.listed_at), ("git", self.resolved_at)]
            .into_iter()
            .filter_map(|(side, at)| {
                at.map(|at| {
                    format!(
                        "{} {} ago",
                        side,
                        short_duration(at.elapsed().as_secs() as i64)
                    )
                })
            })
            .collect();

        parts.join(" · ")
    }

    pub fn matching_namespaces(&self) -> usize {
        self.ordered_indices().len()
    }

    // ------------------------------------------------------------------------ verdicts

    pub fn remote(&self, namespace: &str) -> Option<&RemoteState> {
        self.remotes.get(&self.mode)?.get(namespace)
    }

    pub fn container_verdict(&self, namespace: &str, container: &MyContainer) -> Verdict {
        judge(container.image_ref().as_ref(), self.remote(namespace))
    }

    pub fn pod_verdict(&self, namespace: &str, pod: &MyPod) -> Verdict {
        if pod.finished {
            return Verdict::Unknown(Unresolved::Finished);
        }

        Verdict::of_parts(
            pod.containers
                .iter()
                .map(|container| self.container_verdict(namespace, container)),
        )
    }

    pub fn namespace_verdict(&self, entry: &Entry) -> Verdict {
        match &entry.pods {
            PodsState::Failed(_) => Verdict::Failed(Failure::Kube),
            PodsState::Listing => Verdict::Resolving,
            PodsState::Ready(pods) if pods.is_empty() => self.reference_verdict(&entry.name),
            PodsState::Ready(pods) => {
                Verdict::of_parts(pods.iter().map(|pod| self.pod_verdict(&entry.name, pod)))
            }
        }
    }

    /// Verdict a namespace gets from its repository alone, used when there are no pods to
    /// judge.
    fn reference_verdict(&self, namespace: &str) -> Verdict {
        match self.remote(namespace) {
            Some(RemoteState::Ready(_)) => Verdict::Unknown(Unresolved::Empty),
            other => judge(None, other),
        }
    }

    // ------------------------------------------------------------------------ view rows

    pub fn overview_rows(&self) -> Vec<NsRow> {
        let names = self.names();

        self.ordered_indices()
            .into_iter()
            .map(|index| self.ns_row(index, &names))
            .collect()
    }

    /// The selected namespace as the tables show it.
    pub fn selected_row(&self) -> Option<NsRow> {
        let pick = self.ns_pick.as_deref()?;
        let index = self.entries.iter().position(|entry| entry.name == pick)?;

        Some(self.ns_row(index, &self.names()))
    }

    /// The same namespaces, grouped by family, the way the detail screen lists them. The
    /// heading carries the whole prefix the family shares, so the rows below it do not have
    /// to repeat it.
    pub fn mini_rows(&self) -> Vec<MiniRow> {
        let mut rows: Vec<MiniRow> = vec![];
        let mut family: Option<String> = None;

        let names = self.names();

        for index in self.family_order() {
            let row = self.ns_row(index, &names);
            let label = family_label(shared_prefix(&row.name, &names));

            if family.as_deref() != Some(label.as_str()) {
                rows.push(MiniRow::Family(label.clone()));
                family = Some(label);
            }

            rows.push(MiniRow::Namespace(row));
        }

        rows
    }

    /// Where the cursor sits among the rows of the detail screen's namespace list, which
    /// also holds unselectable family headings.
    pub fn mini_selected(&self) -> Option<usize> {
        let pick = self.ns_pick.as_deref()?;

        self.mini_rows().iter().position(|row| match row {
            MiniRow::Namespace(ns) => ns.name == pick,
            MiniRow::Family(_) => false,
        })
    }

    pub fn pod_rows(&self) -> Vec<PodRow> {
        let Some(entry) = self.selected_entry() else {
            return vec![];
        };
        let now = Utc::now();

        entry
            .pod_list()
            .iter()
            .map(|pod| {
                let parts = split_pod_name(&pod.name, &entry.name);

                PodRow {
                    prefix: parts.prefix.to_string(),
                    replicaset: parts.replicaset.to_string(),
                    suffix: parts.suffix.to_string(),
                    verdict: self.pod_verdict(&entry.name, pod),
                    reference: self
                        .pod_reference(&entry.name, pod)
                        .map(|r| r.label())
                        .unwrap_or_else(|| MISSING.to_string()),
                    status: pod.status.clone(),
                    restarts: pod.restarts,
                    age: pod.age(now).unwrap_or_else(|| MISSING.to_string()),
                    readiness: pod.readiness(),
                }
            })
            .collect()
    }

    pub fn container_rows(&self) -> Vec<ContainerRow> {
        let Some(entry) = self.selected_entry() else {
            return vec![];
        };
        let Some(pod) = self.selected_pod() else {
            return vec![];
        };

        pod.containers
            .iter()
            .map(|container| {
                let verdict = match pod.finished {
                    true => Verdict::Unknown(Unresolved::Finished),
                    false => self.container_verdict(&entry.name, container),
                };

                ContainerRow {
                    name: container.name.clone(),
                    tag: container
                        .tag()
                        .map(str::to_string)
                        .unwrap_or_else(|| MISSING.to_string()),
                    verdict,
                    why: self.explain(&entry.name, verdict),
                }
            })
            .collect()
    }

    fn ns_row(&self, index: usize, names: &[String]) -> NsRow {
        let entry = &self.entries[index];
        let (deployed, hidden_deployed) = self.deployed(entry);

        NsRow {
            prefix: shared_prefix(&entry.name, names).chars().count(),
            name: entry.name.clone(),
            verdict: self.namespace_verdict(entry),
            pods: match &entry.pods {
                PodsState::Ready(pods) => Some(pods.len()),
                _ => None,
            },
            deployed,
            hidden_deployed,
            reference: self.reference_label(&entry.name),
            note: self.note_for(entry),
        }
    }

    /// What the pods of a namespace are running, grouped. A pod nobody can judge is left
    /// out — somebody else's workload sharing the namespace is not a second version of
    /// ours — but only while another pod can be judged: when none can, showing what they
    /// run is all the column has to offer.
    fn deployed(&self, entry: &Entry) -> (Vec<Deployed>, usize) {
        let mut groups: Vec<Deployed> = vec![];
        let judged = entry
            .pod_list()
            .iter()
            .any(|pod| !matches!(self.pod_verdict(&entry.name, pod), Verdict::Unknown(_)));

        for pod in entry.pod_list() {
            let Some(label) = self.pod_reference(&entry.name, pod).map(|r| r.label()) else {
                continue;
            };
            let verdict = self.pod_verdict(&entry.name, pod);

            if judged && matches!(verdict, Verdict::Unknown(_)) {
                continue;
            }

            match groups.iter_mut().find(|group| group.label == label) {
                Some(group) => {
                    group.count += 1;
                    group.verdict = Verdict::of_parts([group.verdict, verdict]);
                }
                None => groups.push(Deployed {
                    label,
                    count: 1,
                    verdict,
                }),
            }
        }

        groups.sort_by(|a, b| b.count.cmp(&a.count).then(a.label.cmp(&b.label)));
        let hidden = groups.len().saturating_sub(DEPLOYED_GROUPS);
        groups.truncate(DEPLOYED_GROUPS);

        (groups, hidden)
    }

    /// Reference a pod stands for: the one of its containers that says the most about the
    /// release. Taking the first container instead would let a service mesh proxy injected
    /// ahead of the application speak for the whole pod.
    ///
    /// A pod where nothing can speak — a foreign workload, or one pinned by digest — stands
    /// for nothing, and saying so beats naming a version that means something else entirely.
    fn pod_reference(&self, namespace: &str, pod: &MyPod) -> Option<ImageRef> {
        let remote = self.remote(namespace);

        pod.containers
            .iter()
            .filter_map(|container| {
                let image_ref = container.image_ref()?;
                let rank = reference_rank(judge(Some(&image_ref), remote))?;

                Some((rank, image_ref))
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, image_ref)| image_ref)
    }

    /// What every namespace's pods carry, when they all carry the same kind of reference.
    pub fn deployed_kind(&self) -> RefKind {
        self.kind_of(self.entries.iter())
    }

    /// The same for the selected namespace alone.
    pub fn selected_kind(&self) -> RefKind {
        self.kind_of(self.selected_entry().into_iter())
    }

    fn kind_of<'a>(&self, entries: impl Iterator<Item = &'a Entry>) -> RefKind {
        let mut tags = false;
        let mut commits = false;

        for entry in entries {
            for pod in entry.pod_list() {
                match self.pod_reference(&entry.name, pod) {
                    Some(ImageRef::Tag(_)) => tags = true,
                    Some(ImageRef::Commit(_)) => commits = true,
                    None => (),
                }
            }
        }

        match (tags, commits) {
            (true, false) => RefKind::Tags,
            (false, true) => RefKind::Commits,
            (true, true) => RefKind::Mixed,
            // nothing has arrived yet: name the column after what the mode expects
            (false, false) => match self.mode {
                Mode::Commit => RefKind::Commits,
                Mode::Tag => RefKind::Tags,
            },
        }
    }

    pub fn reference_label(&self, namespace: &str) -> String {
        match self.remote(namespace) {
            Some(RemoteState::Ready(target)) => {
                target.name.clone().unwrap_or_else(|| target.short())
            }
            None | Some(RemoteState::Loading) => "resolving…".to_string(),
            _ => MISSING.to_string(),
        }
    }

    /// What the pods of the selected namespace are compared against, spelled out for the
    /// header of the detail screen.
    pub fn reference_detail(&self) -> (String, String) {
        let Some(entry) = self.selected_entry() else {
            return (MISSING.to_string(), String::new());
        };

        match self.remote(&entry.name) {
            Some(RemoteState::Ready(target)) => (
                target.label(),
                match self.mode {
                    Mode::Commit => "tip of the default branch".to_string(),
                    Mode::Tag => "latest version tag".to_string(),
                },
            ),
            Some(RemoteState::Loading) | None => ("resolving…".to_string(), String::new()),
            Some(RemoteState::Unconfigured) => (
                MISSING.to_string(),
                "namespace has no entry in repos.json".to_string(),
            ),
            Some(RemoteState::NoTags) => (
                MISSING.to_string(),
                "repository has no tags yet".to_string(),
            ),
            Some(RemoteState::Failed(reason)) => (MISSING.to_string(), reason.clone()),
        }
    }

    pub fn repo_url(&self, namespace: &str) -> Option<&str> {
        Some(self.repos.get_repo_by_name(namespace)?.url.as_str())
    }

    fn note_for(&self, entry: &Entry) -> String {
        match &entry.pods {
            PodsState::Listing => "listing pods…".to_string(),
            PodsState::Failed(reason) => format!("cannot list pods · {}", reason),
            PodsState::Ready(pods) => match self.namespace_verdict(entry) {
                Verdict::Behind => {
                    let behind = pods
                        .iter()
                        .filter(|pod| self.pod_verdict(&entry.name, pod) == Verdict::Behind)
                        .count();
                    let (groups, hidden) = self.deployed(entry);
                    // the deployed column already shows the split; the word only names it
                    let mixed = match groups.len() + hidden > 1 {
                        true => " · mixed",
                        false => "",
                    };

                    format!("{} of {} pods behind{}", behind, pods.len(), mixed)
                }
                Verdict::Failed(_) => match self.remote(&entry.name) {
                    Some(RemoteState::Failed(reason)) => format!("git · {}", reason),
                    _ => "git ls-remote failed".to_string(),
                },
                Verdict::Resolving => "git ls-remote in flight".to_string(),
                verdict @ Verdict::Unknown(Unresolved::Finished) => {
                    format!("{} · no running pods", verdict.code())
                }
                verdict @ Verdict::Unknown(reason) => {
                    format!("{} · {}", verdict.code(), reason_text(reason))
                }
                Verdict::InSync => String::new(),
            },
        }
    }

    /// Plain words for a container's verdict — the answer to "why is this row grey".
    fn explain(&self, namespace: &str, verdict: Verdict) -> String {
        match verdict {
            Verdict::InSync => format!("matches {}", self.reference_label(namespace)),
            Verdict::Behind => format!("behind · reference is {}", self.reference_label(namespace)),
            Verdict::Resolving => "resolving the reference".to_string(),
            Verdict::Failed(_) => match self.remote(namespace) {
                Some(RemoteState::Failed(reason)) => reason.clone(),
                _ => "lookup failed".to_string(),
            },
            Verdict::Unknown(reason) => format!("{} · {}", verdict.code(), reason_text(reason)),
        }
    }

    // -------------------------------------------------------------------------- footer

    /// What the footer says: the last action, or why the selected namespace cannot be
    /// judged. Actions expire, standing problems do not.
    pub fn note(&self) -> Option<Note> {
        if let Some(note) = self.reported() {
            return Some(Note {
                kind: note.kind,
                text: note.text.clone(),
                retry: note.retry,
                standing: false,
            });
        }

        let entry = self.selected_entry()?;

        if let PodsState::Failed(reason) = &entry.pods {
            return Some(Note {
                kind: NoteKind::Error,
                text: format!("{} · cannot list pods: {}", entry.name, reason),
                retry: true,
                standing: true,
            });
        }

        match self.remote(&entry.name) {
            Some(RemoteState::Failed(reason)) => Some(Note {
                kind: NoteKind::Error,
                text: format!("{} · git ls-remote: {}", entry.name, reason),
                retry: true,
                standing: true,
            }),
            Some(RemoteState::Unconfigured) => Some(Note {
                kind: NoteKind::Warn,
                text: format!(
                    "{} is missing in repos.json — nothing to compare",
                    entry.name
                ),
                retry: false,
                standing: true,
            }),
            Some(RemoteState::NoTags) => Some(Note {
                kind: NoteKind::Warn,
                text: format!(
                    "{} has no tags — try {} mode",
                    entry.name,
                    self.mode.toggled().label()
                ),
                retry: false,
                standing: true,
            }),
            _ => None,
        }
    }

    /// The reported action, while it is still worth showing.
    fn reported(&self) -> Option<&Note> {
        self.note
            .as_ref()
            .filter(|(_, at)| at.elapsed() < NOTE_TTL)
            .map(|(note, _)| note)
    }

    /// Gives up the reported action. An expired one is already invisible, so giving it up
    /// must not count as having done something — Esc would be swallowed for nothing.
    pub fn dismiss_note(&mut self) -> bool {
        let showing = self.reported().is_some();
        self.note = None;

        showing
    }

    /// One line about the selected container, shown on the detail screen.
    pub fn selection_summary(&self) -> Option<String> {
        let entry = self.selected_entry()?;
        let container = self.selected_container()?;
        let verdict = self.container_verdict(&entry.name, container);
        let running = container
            .tag()
            .map(str::to_string)
            .unwrap_or_else(|| MISSING.to_string());

        Some(match verdict {
            Verdict::Behind => format!(
                "{} runs {} → reference is {}",
                container.name,
                running,
                self.reference_label(&entry.name)
            ),
            Verdict::InSync => format!("{} runs the reference {}", container.name, running),
            _ => format!(
                "{} {} · {}",
                container.name,
                running,
                self.explain(&entry.name, verdict)
            ),
        })
    }

    // ------------------------------------------------------------------------ selection

    pub fn selected_entry(&self) -> Option<&Entry> {
        let pick = self.ns_pick.as_deref()?;

        self.entries.iter().find(|entry| entry.name == pick)
    }

    pub fn selected_pod(&self) -> Option<&MyPod> {
        let pick = self.pod_pick.as_deref()?;

        self.selected_entry()?
            .pod_list()
            .iter()
            .find(|pod| pod.name == pick)
    }

    pub fn selected_container(&self) -> Option<&MyContainer> {
        let pick = self.cont_pick.as_deref()?;

        self.selected_pod()?
            .containers
            .iter()
            .find(|container| container.name == pick)
    }

    pub fn ns_state(&mut self) -> &mut TableState {
        &mut self.ns_state
    }

    pub fn pod_state(&mut self) -> &mut TableState {
        &mut self.pod_state
    }

    pub fn cont_state(&mut self) -> &mut TableState {
        &mut self.cont_state
    }

    pub fn ns_position(&self) -> Option<usize> {
        let pick = self.ns_pick.as_deref()?;

        self.ordered_names().iter().position(|name| name == pick)
    }

    pub fn pod_position(&self) -> Option<usize> {
        let pick = self.pod_pick.as_deref()?;

        self.selected_entry()?
            .pod_list()
            .iter()
            .position(|pod| pod.name == pick)
    }

    pub fn container_position(&self) -> Option<usize> {
        let pick = self.cont_pick.as_deref()?;

        self.selected_pod()?
            .containers
            .iter()
            .position(|container| container.name == pick)
    }

    pub fn move_selection(&mut self, delta: isize) {
        let rows = self.active_rows();
        if rows.is_empty() {
            return;
        }

        let current = self
            .active_pick()
            .and_then(|pick| rows.iter().position(|row| *row == pick))
            .unwrap_or(0) as isize;
        let next = current
            .saturating_add(delta)
            .clamp(0, rows.len() as isize - 1) as usize;

        self.set_active_pick(rows[next].clone());
    }

    pub fn focus_next(&mut self) {
        self.pane = match self.pane {
            Pane::Namespaces => Pane::Pods,
            Pane::Pods | Pane::Containers => Pane::Containers,
        };
    }

    pub fn focus_prev(&mut self) {
        self.pane = match self.pane {
            Pane::Namespaces | Pane::Pods => Pane::Namespaces,
            Pane::Containers => Pane::Pods,
        };
    }

    pub fn focus_containers(&mut self) {
        self.pane = Pane::Containers;
    }

    /// Enter: the overview opens a namespace, the detail screen opens a commit.
    pub fn activate(&mut self) {
        match self.screen {
            Screen::Overview => self.open_namespace(),
            Screen::Detail => self.open_commit(),
        }
    }

    pub fn open_namespace(&mut self) {
        if self.ns_pick.is_none() {
            return;
        }

        self.screen = Screen::Detail;
        self.pane = Pane::Pods;
    }

    /// Esc: gives up whatever is on top — a message, then the detail screen, then the tool.
    pub fn back(&mut self) {
        if self.dismiss_note() {
            return;
        }

        match self.screen {
            Screen::Detail => {
                self.screen = Screen::Overview;
                self.pane = Pane::Pods;
            }
            Screen::Overview => self.quit(),
        }
    }

    // ------------------------------------------------------------------------- commands

    /// Switching modes reuses whatever was already resolved and only asks for the rest.
    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.toggled();
        self.note = None;
        self.request_missing();
    }

    pub fn cycle_filter(&mut self) {
        self.filter = self.filter.next();
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
    }

    /// Search filters the namespace list, so it is only allowed while that list is the one
    /// the user is looking at — filtering a list that is off screen would move the selection
    /// under them.
    pub fn start_search(&mut self) {
        if self.searchable() {
            self.input = Input::Search;
        }
    }

    pub fn searchable(&self) -> bool {
        match self.screen {
            Screen::Overview => true,
            Screen::Detail => self.pane == Pane::Namespaces && !self.narrow,
        }
    }

    pub fn search_push(&mut self, symbol: char) {
        self.query.push(symbol);
    }

    pub fn search_pop(&mut self) {
        self.query.pop();
    }

    /// Leaving search either keeps the typed filter or drops it.
    pub fn end_search(&mut self, keep: bool) {
        self.input = Input::Normal;

        if !keep {
            self.query.clear();
        }
    }

    pub fn toggle_help(&mut self) {
        self.input = match self.input {
            Input::Help => Input::Normal,
            _ => Input::Help,
        };
    }

    /// Asks the selected namespace again — `git` for its reference, and Kubernetes for its
    /// pods when that is the side that failed. The footer offers `r` for both, so both have
    /// to happen.
    pub fn refresh_selected(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let name = entry.name.clone();
        let unlisted = matches!(entry.pods, PodsState::Failed(_));

        if unlisted {
            // the round stays as it is: nothing is in flight for a namespace that failed,
            // and bumping it would throw away the answers still coming for the others
            let round = self.listing_round;

            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.name == name) {
                entry.pods = PodsState::Listing;
            }

            self.spawn_listing(round, name.clone());
        }

        // a second request would spawn another thread and could land out of order,
        // overwriting the newer answer with the older one
        if !matches!(self.remote(&name), Some(RemoteState::Loading)) {
            self.request(&name);
        }

        self.note = None;
    }

    /// Asks Kubernetes for the pods of every namespace again.
    pub fn relist_pods(&mut self) {
        self.listing_round += 1;
        let round = self.listing_round;

        for entry in &mut self.entries {
            entry.pods = PodsState::Listing;
        }

        for name in self.names() {
            self.spawn_listing(round, name);
        }
    }

    fn spawn_listing(&self, round: u64, name: String) {
        let tx = self.pods_tx.clone();
        let client = self.kube.clone();

        tokio::spawn(async move {
            let pods = MyPod::get_pods_by_ns(&client, &name).await;
            let _ = tx.send((round, name, pods));
        });
    }

    fn open_commit(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let namespace = entry.name.clone();

        let image_ref = match self.pane {
            Pane::Containers => self.selected_container().and_then(|c| c.image_ref()),
            _ => self
                .selected_pod()
                .and_then(|pod| self.pod_reference(&namespace, pod)),
        };

        let Some(image_ref) = image_ref else {
            self.report(NoteKind::Warn, "nothing here carries a commit".to_string());
            return;
        };

        // a release image names a tag, so its commit is only known once tags are fetched
        let hash = match self.remote(&namespace) {
            Some(RemoteState::Ready(target)) => target.commit_of(&image_ref),
            _ => None,
        };

        let Some(hash) = hash else {
            self.report(
                NoteKind::Warn,
                format!(
                    "cannot resolve {} to a commit — try {} mode",
                    image_ref.as_str(),
                    self.mode.toggled().label()
                ),
            );
            return;
        };

        let Some(url) = self.repo_url(&namespace).map(str::to_string) else {
            self.report(
                NoteKind::Warn,
                format!("{} is missing in repos.json", namespace),
            );
            return;
        };

        match open_with_hash(&url, &hash) {
            Ok(_) => self.report(NoteKind::Ok, format!("opened {}", commit_url(&url, &hash))),
            Err(e) => self.report(NoteKind::Error, format!("cannot open browser: {}", e)),
        }
    }

    fn report(&mut self, kind: NoteKind, text: String) {
        self.note = Some((
            Note {
                kind,
                text,
                retry: false,
                standing: false,
            },
            Instant::now(),
        ));
    }

    // -------------------------------------------------------------- background plumbing

    /// Asks for every namespace the current mode has no answer for yet.
    fn request_missing(&mut self) {
        let pending: Vec<String> = self
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .filter(|name| self.remote(name).is_none())
            .collect();

        for name in pending {
            self.request(&name);
        }
    }

    fn request(&mut self, namespace: &str) {
        let mode = self.mode;

        match self.repos.get_repo_by_name(namespace) {
            Some(repo) => {
                let url = repo.url.clone();
                self.set_remote(namespace, RemoteState::Loading);
                spawn_lookup(self.lookups_tx.clone(), mode, namespace.to_string(), url);
            }
            None => self.set_remote(namespace, RemoteState::Unconfigured),
        }
    }

    fn drain_lookups(&mut self) {
        while let Ok((mode, name, result)) = self.lookups_rx.try_recv() {
            let state = match result {
                Ok(target) => RemoteState::Ready(target),
                Err(LookupError::NoTags) => RemoteState::NoTags,
                Err(LookupError::Git(reason)) => RemoteState::Failed(reason),
            };

            self.remotes.entry(mode).or_default().insert(name, state);
            self.resolved_at = Some(Instant::now());
        }
    }

    fn drain_pods(&mut self) {
        while let Ok((round, name, result)) = self.pods_rx.try_recv() {
            self.accept_pods(round, &name, result);
        }
    }

    fn accept_pods(&mut self, round: u64, name: &str, result: Result<Vec<MyPod>, MsgError>) {
        // an answer from an earlier round would put a stale list of pods back on screen
        if round != self.listing_round {
            return;
        }

        let state = match result {
            Ok(pods) => PodsState::Ready(pods),
            Err(e) => PodsState::Failed(e.details),
        };

        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.name == name) {
            entry.pods = state;
        }

        self.listed_at = Some(Instant::now());
    }

    fn set_remote(&mut self, namespace: &str, state: RemoteState) {
        self.remotes
            .entry(self.mode)
            .or_default()
            .insert(namespace.to_string(), state);
    }

    // ------------------------------------------------------------------------- ordering

    /// Namespaces the current filter and search leave, in the order of the active screen.
    fn ordered_indices(&self) -> Vec<usize> {
        match self.screen {
            Screen::Overview => self.sorted_order(),
            Screen::Detail => self.family_order(),
        }
    }

    fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    fn ordered_names(&self) -> Vec<String> {
        self.ordered_indices()
            .into_iter()
            .map(|index| self.entries[index].name.clone())
            .collect()
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();

        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name.to_lowercase().contains(&query))
            .filter(|(_, entry)| self.filter.accepts(self.namespace_verdict(entry)))
            .map(|(index, _)| index)
            .collect()
    }

    fn sorted_order(&self) -> Vec<usize> {
        let mut indices = self.visible_indices();

        indices.sort_by(|a, b| {
            let (left, right) = (&self.entries[*a], &self.entries[*b]);

            match self.sort {
                Sort::Status => self
                    .namespace_verdict(left)
                    .severity()
                    .cmp(&self.namespace_verdict(right).severity())
                    .then(left.name.cmp(&right.name)),
                Sort::Name => left.name.cmp(&right.name),
                Sort::Pods => right
                    .pod_list()
                    .len()
                    .cmp(&left.pod_list().len())
                    .then(left.name.cmp(&right.name)),
            }
        });

        indices
    }

    /// Grouped by the family a namespace belongs to, worst first inside a family.
    fn family_order(&self) -> Vec<usize> {
        let names = self.names();
        let mut indices = self.visible_indices();

        indices.sort_by(|a, b| {
            let (left, right) = (&self.entries[*a], &self.entries[*b]);
            let left_family = family_prefix(&left.name, &names);
            let right_family = family_prefix(&right.name, &names);

            // families in alphabetical order, the unfamilied ones last
            left_family
                .is_empty()
                .cmp(&right_family.is_empty())
                .then(left_family.cmp(right_family))
                .then(
                    self.namespace_verdict(left)
                        .severity()
                        .cmp(&self.namespace_verdict(right).severity()),
                )
                .then(left.name.cmp(&right.name))
        });

        indices
    }

    fn active_rows(&self) -> Vec<String> {
        match (self.screen, self.pane) {
            (Screen::Overview, _) | (Screen::Detail, Pane::Namespaces) => self.ordered_names(),
            (Screen::Detail, Pane::Pods) => self
                .selected_entry()
                .map(|entry| entry.pod_list().iter().map(|p| p.name.clone()).collect())
                .unwrap_or_default(),
            (Screen::Detail, Pane::Containers) => self
                .selected_pod()
                .map(|pod| pod.containers.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default(),
        }
    }

    fn active_pick(&self) -> Option<String> {
        match (self.screen, self.pane) {
            (Screen::Overview, _) | (Screen::Detail, Pane::Namespaces) => self.ns_pick.clone(),
            (Screen::Detail, Pane::Pods) => self.pod_pick.clone(),
            (Screen::Detail, Pane::Containers) => self.cont_pick.clone(),
        }
    }

    fn set_active_pick(&mut self, pick: String) {
        match (self.screen, self.pane) {
            (Screen::Overview, _) | (Screen::Detail, Pane::Namespaces) => {
                if self.ns_pick.as_deref() != Some(pick.as_str()) {
                    self.pod_pick = None;
                    self.cont_pick = None;
                }
                self.ns_pick = Some(pick);
            }
            (Screen::Detail, Pane::Pods) => {
                if self.pod_pick.as_deref() != Some(pick.as_str()) {
                    self.cont_pick = None;
                }
                self.pod_pick = Some(pick);
            }
            (Screen::Detail, Pane::Containers) => self.cont_pick = Some(pick),
        }
    }

    /// Keeps every cursor on a row that exists, which matters because rows arrive, get
    /// filtered out and get reordered while the tool is open.
    fn normalize(&mut self) {
        let names = self.ordered_names();
        if !names
            .iter()
            .any(|name| self.ns_pick.as_deref() == Some(name))
        {
            self.ns_pick = names.first().cloned();
            self.pod_pick = None;
            self.cont_pick = None;
        }

        let pods: Vec<String> = self
            .selected_entry()
            .map(|entry| entry.pod_list().iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        if !pods
            .iter()
            .any(|name| self.pod_pick.as_deref() == Some(name))
        {
            self.pod_pick = pods.first().cloned();
            self.cont_pick = None;
        }

        let containers: Vec<String> = self
            .selected_pod()
            .map(|pod| pod.containers.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default();
        if !containers
            .iter()
            .any(|name| self.cont_pick.as_deref() == Some(name))
        {
            self.cont_pick = containers.first().cloned();
        }
    }
}

fn family_label(prefix: &str) -> String {
    match prefix.trim_end_matches('-') {
        "" => "other".to_string(),
        family => family.to_string(),
    }
}

/// How much a container's reference is worth as the pod's: one that could be compared beats
/// one still being looked up, which beats one this mode cannot resolve at all. `None` for a
/// container that says nothing about this repository.
fn reference_rank(verdict: Verdict) -> Option<u8> {
    match verdict {
        Verdict::InSync | Verdict::Behind => Some(0),
        Verdict::Resolving | Verdict::Failed(_) => Some(1),
        Verdict::Unknown(Unresolved::WrongMode) => Some(2),
        Verdict::Unknown(_) => None,
    }
}

fn reason_text(reason: Unresolved) -> &'static str {
    match reason {
        Unresolved::Sidecar => "not built from this repository",
        Unresolved::Digest => "image carries no commit",
        Unresolved::NoConfig => "missing in repos.json",
        Unresolved::NoTags => "repository has no tags",
        Unresolved::WrongMode => "commit mode cannot resolve a version — try tag mode",
        Unresolved::Finished => "the pod has already run its course",
        Unresolved::Empty => "no pods to compare",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::remote::Target;
    use crate::storage::common::Repo;
    use kube::Config;
    use speculoos::prelude::*;

    const GATEWAY: &str = "orchard-gateway";
    const LEDGER: &str = "orchard-ledger-api";
    const MAILER: &str = "tulip-mailer";
    const LEFTOVER: &str = "91bd7e40-8103771";

    fn client() -> Client {
        Client::try_from(Config::new("http://127.0.0.1:1".parse().unwrap())).unwrap()
    }

    fn repos(names: &[&str]) -> JsonStorage {
        JsonStorage::from_repos(
            names
                .iter()
                .map(|name| Repo {
                    name: name.to_string(),
                    url: format!("https://git.example.com/{}", name),
                })
                .collect(),
        )
    }

    fn app(names: &[&str]) -> App {
        let mut app = App::new(
            names.iter().map(|n| n.to_string()).collect(),
            repos(names),
            Mode::Commit,
            Cluster::default(),
            client(),
        );

        // App::new starts a real listing against an address nobody answers; moving on a
        // round makes its eventual failures stale, so the tests decide what the state is
        app.listing_round += 1;

        app
    }

    fn pod(namespace: &str, name: &str, tag: &str) -> MyPod {
        MyPod {
            name: name.to_string(),
            namespace: namespace.to_string(),
            containers: vec![MyContainer {
                name: "app".to_string(),
                image: format!("registry.example/{}:{}", namespace, tag),
            }],
            status: "Running".to_string(),
            restarts: 0,
            ready: 1,
            created: None,
            finished: false,
        }
    }

    /// Puts a namespace in the state it would reach after both lookups came back.
    fn settle(app: &mut App, namespace: &str, deployed: &str, reference: &str) {
        app.accept_pods(
            app.listing_round,
            namespace,
            Ok(vec![pod(
                namespace,
                &format!("{}-abc12-x1y2z", namespace),
                deployed,
            )]),
        );
        app.set_remote(
            namespace,
            RemoteState::Ready(Target {
                commit: reference.to_string(),
                ..Target::default()
            }),
        );
    }

    fn in_sync(app: &mut App, namespace: &str) {
        settle(
            app,
            namespace,
            "4f1c0f2c-8103771",
            "4f1c0f2c8b3d4e5a6f708192a3b4c5d6e7f80912",
        );
    }

    fn behind(app: &mut App, namespace: &str) {
        settle(
            app,
            namespace,
            "91bd7e40-8103771",
            "4f1c0f2c8b3d4e5a6f708192a3b4c5d6e7f80912",
        );
    }

    /// Somebody else's workload sharing the namespace: a tag this repository never had.
    fn foreign(namespace: &str) -> MyPod {
        MyPod {
            name: format!("{}-mesh-ingress-7c1b9-vd6np", namespace),
            namespace: namespace.to_string(),
            containers: vec![MyContainer {
                name: "mesh-ingress".to_string(),
                image: "registry.example/mesh-ingress:1.21".to_string(),
            }],
            status: "Running".to_string(),
            restarts: 0,
            ready: 1,
            created: None,
            finished: false,
        }
    }

    #[tokio::test]
    async fn foreign_pod_does_not_hold_the_namespace_back() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        let mut pods = vec![foreign(GATEWAY)];
        pods.push(pod(
            GATEWAY,
            "orchard-gateway-abc12-x1y2z",
            "4f1c0f2c-8103771",
        ));
        app.accept_pods(app.listing_round, GATEWAY, Ok(pods));

        asserting!("a pod that is not ours must not make the namespace unjudgeable")
            .that(
                &app.selected_entry()
                    .map(|entry| app.namespace_verdict(entry)),
            )
            .is_equal_to(Some(Verdict::InSync));
    }

    #[tokio::test]
    async fn foreign_pod_is_not_counted_as_a_deployed_commit() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        let mut pods = vec![foreign(GATEWAY)];
        pods.push(pod(
            GATEWAY,
            "orchard-gateway-abc12-x1y2z",
            "4f1c0f2c-8103771",
        ));
        app.accept_pods(app.listing_round, GATEWAY, Ok(pods));

        asserting!("its tag would otherwise read as a second commit of ours")
            .that(&app.overview_rows().first().map(|row| row.deployed.len()))
            .is_equal_to(Some(1));
    }

    #[tokio::test]
    async fn namespace_of_nothing_but_foreign_pods_says_why() {
        let release = "9d02f4ab3c1d5e7f8091a2b3c4d5e6f708192a3b";
        let mut app = app(&[GATEWAY]);
        app.set_remote(
            GATEWAY,
            RemoteState::Ready(Target {
                commit: release.to_string(),
                name: Some("v3.7.1".to_string()),
                tags: HashMap::from([("v3.7.1".to_string(), release.to_string())]),
            }),
        );
        app.accept_pods(app.listing_round, GATEWAY, Ok(vec![foreign(GATEWAY)]));

        asserting!("the repository has tags, and this image is built from none of them")
            .that(
                &app.selected_entry()
                    .map(|entry| app.namespace_verdict(entry)),
            )
            .is_equal_to(Some(Verdict::Unknown(Unresolved::Sidecar)));
    }

    #[tokio::test]
    async fn release_image_in_commit_mode_says_which_mode_would_work() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        app.accept_pods(
            app.listing_round,
            GATEWAY,
            Ok(vec![pod(
                GATEWAY,
                "orchard-gateway-abc12-x1y2z",
                "v3.7.1-8203114",
            )]),
        );

        asserting!("production runs releases, and commit mode cannot resolve one")
            .that(
                &app.selected_entry()
                    .map(|entry| app.namespace_verdict(entry)),
            )
            .is_equal_to(Some(Verdict::Unknown(Unresolved::WrongMode)));
    }

    /// What a CronJob leaves behind: it ran an older commit and then stopped.
    /// A pod whose service mesh proxy is injected ahead of the application.
    fn proxied(namespace: &str, tag: &str) -> MyPod {
        let mut running = pod(namespace, &format!("{}-abc12-x1y2z", namespace), tag);
        running.containers.insert(
            0,
            MyContainer {
                name: "mesh-proxy".to_string(),
                image: "registry.example/mesh-proxy:1.26.2".to_string(),
            },
        );

        running
    }

    fn finished(namespace: &str, tag: &str) -> MyPod {
        MyPod {
            finished: true,
            status: "Completed".to_string(),
            ready: 0,
            ..pod(
                namespace,
                &format!("{}-cleaner-29749830-9fmrv", namespace),
                tag,
            )
        }
    }

    #[tokio::test]
    async fn pod_that_has_finished_does_not_make_the_namespace_lag() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        let mut pods = vec![finished(GATEWAY, LEFTOVER)];
        pods.push(pod(
            GATEWAY,
            "orchard-gateway-abc12-x1y2z",
            "4f1c0f2c-8103771",
        ));
        app.accept_pods(app.listing_round, GATEWAY, Ok(pods));

        asserting!("it ran the code that was current then and deploys nothing now")
            .that(
                &app.selected_entry()
                    .map(|entry| app.namespace_verdict(entry)),
            )
            .is_equal_to(Some(Verdict::InSync));
    }

    #[tokio::test]
    async fn pod_that_has_finished_is_not_counted_as_a_deployed_commit() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        let mut pods = vec![finished(GATEWAY, LEFTOVER)];
        pods.push(pod(
            GATEWAY,
            "orchard-gateway-abc12-x1y2z",
            "4f1c0f2c-8103771",
        ));
        app.accept_pods(app.listing_round, GATEWAY, Ok(pods));

        asserting!("its commit would otherwise read as a rollout that stopped half way")
            .that(&app.overview_rows().first().map(|row| row.deployed.len()))
            .is_equal_to(Some(1));
    }

    #[tokio::test]
    async fn namespace_where_nothing_runs_any_more_says_so() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        app.accept_pods(
            app.listing_round,
            GATEWAY,
            Ok(vec![finished(GATEWAY, LEFTOVER)]),
        );

        assert_that!(app
            .selected_entry()
            .map(|entry| app.namespace_verdict(entry)))
        .is_equal_to(Some(Verdict::Unknown(Unresolved::Finished)));
    }

    #[tokio::test]
    async fn pod_that_has_finished_still_shows_what_it_ran() {
        let ran = "91bd7e40";
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        app.accept_pods(
            app.listing_round,
            GATEWAY,
            Ok(vec![finished(GATEWAY, LEFTOVER)]),
        );
        app.tick();

        asserting!("keeping it out of the verdict must not hide it")
            .that(&app.pod_rows().first().map(|row| row.reference.clone()))
            .is_equal_to(Some(ran.to_string()));
    }

    #[tokio::test]
    async fn application_speaks_for_the_pod_not_the_proxy_beside_it() {
        let running = "4f1c0f2c";
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        app.accept_pods(
            app.listing_round,
            GATEWAY,
            Ok(vec![proxied(GATEWAY, "4f1c0f2c-8103771")]),
        );

        asserting!("a proxy injected first would otherwise report its own version as ours")
            .that(
                &app.overview_rows()
                    .first()
                    .map(|row| row.deployed[0].label.clone()),
            )
            .is_equal_to(Some(running.to_string()));
    }

    #[tokio::test]
    async fn proxy_beside_the_application_does_not_rename_the_column() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        app.accept_pods(
            app.listing_round,
            GATEWAY,
            Ok(vec![proxied(GATEWAY, "4f1c0f2c-8103771")]),
        );

        assert_that!(app.deployed_kind()).is_equal_to(RefKind::Commits);
    }

    #[tokio::test]
    async fn column_is_named_after_the_commits_it_holds() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);

        assert_that!(app.deployed_kind()).is_equal_to(RefKind::Commits);
    }

    #[tokio::test]
    async fn column_is_named_after_the_tags_it_holds() {
        let mut app = app(&[GATEWAY]);
        in_sync(&mut app, GATEWAY);
        app.accept_pods(
            app.listing_round,
            GATEWAY,
            Ok(vec![pod(
                GATEWAY,
                "orchard-gateway-abc12-x1y2z",
                "v3.7.1-8203114",
            )]),
        );

        asserting!("a cluster deployed from releases carries tags whichever mode is on")
            .that(&app.deployed_kind())
            .is_equal_to(RefKind::Tags);
    }

    #[tokio::test]
    async fn column_that_holds_both_is_named_after_neither() {
        let mut app = app(&[GATEWAY, LEDGER]);
        in_sync(&mut app, GATEWAY);
        in_sync(&mut app, LEDGER);
        app.accept_pods(
            app.listing_round,
            LEDGER,
            Ok(vec![pod(
                LEDGER,
                "orchard-ledger-api-abc12-x1y2z",
                "v3.7.1-8203114",
            )]),
        );

        assert_that!(app.deployed_kind()).is_equal_to(RefKind::Mixed);
    }

    #[tokio::test]
    async fn overview_puts_what_needs_acting_on_first() {
        let mut app = app(&[GATEWAY, LEDGER]);
        in_sync(&mut app, GATEWAY);
        behind(&mut app, LEDGER);

        asserting!("a lagging namespace must not be found by scrolling")
            .that(&app.overview_rows().first().map(|row| row.name.clone()))
            .is_equal_to(Some(LEDGER.to_string()));
    }

    #[tokio::test]
    async fn sorting_by_name_ignores_the_verdict() {
        let mut app = app(&[MAILER, GATEWAY]);
        behind(&mut app, MAILER);
        in_sync(&mut app, GATEWAY);
        app.cycle_sort();

        assert_that!(app.overview_rows().first().map(|row| row.name.clone()))
            .is_equal_to(Some(GATEWAY.to_string()));
    }

    #[tokio::test]
    async fn problems_filter_drops_what_is_in_sync() {
        let mut app = app(&[GATEWAY, LEDGER]);
        in_sync(&mut app, GATEWAY);
        behind(&mut app, LEDGER);
        app.cycle_filter();

        assert_that!(app.overview_rows().len()).is_equal_to(1);
    }

    #[tokio::test]
    async fn search_narrows_the_list_to_what_matches() {
        let mut app = app(&[GATEWAY, LEDGER, MAILER]);
        app.start_search();
        for symbol in "ledger".chars() {
            app.search_push(symbol);
        }
        app.tick();

        assert_that!(app.matching_namespaces()).is_equal_to(1);
    }

    #[tokio::test]
    async fn cursor_stays_on_the_namespace_it_was_on_when_the_order_changes() {
        let mut app = app(&[GATEWAY, LEDGER]);
        in_sync(&mut app, GATEWAY);
        in_sync(&mut app, LEDGER);
        app.tick();
        app.move_selection(1);
        let followed = app.selected_entry().map(|entry| entry.name.clone());

        behind(&mut app, GATEWAY);
        app.tick();

        asserting!("re-sorting under the user must not move their cursor to another row")
            .that(&app.selected_entry().map(|entry| entry.name.clone()))
            .is_equal_to(followed);
    }

    #[tokio::test]
    async fn cursor_leaves_a_namespace_the_filter_hides() {
        let mut app = app(&[GATEWAY, LEDGER]);
        in_sync(&mut app, GATEWAY);
        behind(&mut app, LEDGER);
        app.tick();
        app.cycle_filter();
        app.tick();

        assert_that!(app.selected_entry().map(|entry| entry.name.clone()))
            .is_equal_to(Some(LEDGER.to_string()));
    }

    #[tokio::test]
    async fn answer_from_an_earlier_round_is_dropped() {
        let mut app = app(&[GATEWAY]);
        let stale = app.listing_round;
        app.relist_pods();
        app.accept_pods(
            stale,
            GATEWAY,
            Ok(vec![pod(GATEWAY, "gone-abc12-x1y2z", "4f1c0f2c-8103771")]),
        );

        asserting!("a slow answer from before R must not put back the pods it saw")
            .that(&app.selected_entry().map(|entry| entry.pod_list().len()))
            .is_equal_to(Some(0));
    }

    #[tokio::test]
    async fn retrying_a_namespace_kubernetes_refused_asks_it_again() {
        let mut app = app(&[GATEWAY]);
        app.accept_pods(app.listing_round, GATEWAY, Err(MsgError::new("forbidden")));
        app.tick();
        app.refresh_selected();

        asserting!("the footer offers r for this, so r has to ask Kubernetes and show it")
            .that(
                &app.selected_entry()
                    .map(|entry| matches!(entry.pods, PodsState::Listing)),
            )
            .is_equal_to(Some(true));
    }

    #[tokio::test]
    async fn nothing_reported_means_nothing_to_dismiss() {
        let mut app = app(&[GATEWAY]);

        asserting!("Esc would otherwise be swallowed by an invisible message")
            .that(&app.dismiss_note())
            .is_false();
    }

    #[tokio::test]
    async fn reported_action_is_dismissed_once() {
        let mut app = app(&[GATEWAY]);
        app.report(NoteKind::Ok, "opened".to_string());
        app.dismiss_note();

        assert_that!(app.dismiss_note()).is_false();
    }

    #[tokio::test]
    async fn standing_problem_is_marked_as_one() {
        let mut app = app(&[GATEWAY]);
        app.accept_pods(app.listing_round, GATEWAY, Err(MsgError::new("forbidden")));
        app.tick();

        asserting!("the footer must not offer to dismiss what Esc cannot dismiss")
            .that(&app.note().map(|note| note.standing))
            .is_equal_to(Some(true));
    }

    #[tokio::test]
    async fn namespace_missing_from_the_config_is_not_asked_about() {
        let mut app = App::new(
            vec![MAILER.to_string()],
            repos(&[GATEWAY]),
            Mode::Commit,
            Cluster::default(),
            client(),
        );
        app.tick();

        assert_that!(app.reference_label(MAILER)).is_equal_to(MISSING.to_string());
    }
}
