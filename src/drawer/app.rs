use crate::caller::browser::{commit_url, open_with_hash};
use crate::git::remote::{
    commit_status, spawn_lookup, CommitStatus, LookupResult, Mode, RemoteState,
};
use crate::k8s::pods::{MyContainer, MyPod};
use crate::storage::json_storage::JsonStorage;
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};

pub const KEY_HINTS: &str = "↑↓ move · ←→/Tab pane · Enter open · m mode · r refresh · q quit";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Namespaces,
    Pods,
    Containers,
}

pub struct NamespaceEntry {
    pub name: String,
    pub pods: Vec<MyPod>,
    /// Set when listing pods failed — the namespace stays visible instead of vanishing.
    pub error: Option<String>,
}

pub struct App {
    namespaces: Vec<NamespaceEntry>,
    repos: JsonStorage,
    mode: Mode,
    /// Results are kept per mode, so switching back and forth costs nothing.
    remotes: HashMap<Mode, HashMap<String, RemoteState>>,
    tx: Sender<LookupResult>,
    lookups: Receiver<LookupResult>,
    focus: Focus,
    ns_state: ListState,
    pod_state: ListState,
    cont_state: ListState,
    status: String,
    should_quit: bool,
}

impl App {
    pub fn new(namespaces: Vec<NamespaceEntry>, repos: JsonStorage, mode: Mode) -> App {
        let (tx, lookups) = channel();

        let mut ns_state = ListState::default();
        if !namespaces.is_empty() {
            ns_state.select(Some(0));
        }

        let mut app = App {
            namespaces,
            repos,
            mode,
            remotes: HashMap::new(),
            tx,
            lookups,
            focus: Focus::Namespaces,
            ns_state,
            pod_state: ListState::default(),
            cont_state: ListState::default(),
            status: String::new(),
            should_quit: false,
        };

        app.reset_pods();
        app.request_missing();

        app
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The last action reported to the user, or the state of the background lookups
    /// while there is nothing to report.
    pub fn footer(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }

        if let Some(problem) = self.selected_problem() {
            return problem;
        }

        let pending = self.current_remotes().map_or(0, |remotes| {
            remotes
                .values()
                .filter(|state| matches!(state, RemoteState::Loading))
                .count()
        });

        if pending > 0 {
            return format!("mode: {} · resolving · {} left", self.mode.label(), pending);
        }

        let statuses: Vec<CommitStatus> = self
            .namespaces
            .iter()
            .map(|entry| self.namespace_status(entry))
            .collect();

        let outdated = statuses
            .iter()
            .filter(|s| **s == CommitStatus::Outdated)
            .count();
        let unresolved = statuses
            .iter()
            .filter(|s| **s == CommitStatus::Unknown)
            .count();

        let mut text = format!(
            "mode: {} · {} of {} namespaces outdated",
            self.mode.label(),
            outdated,
            statuses.len()
        );
        if unresolved > 0 {
            text.push_str(&format!(" · {} unresolved", unresolved));
        }

        text
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Switching modes reuses whatever was already resolved and only asks for the rest.
    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.toggled();
        self.status.clear();
        self.request_missing();
    }

    pub fn namespaces(&self) -> &[NamespaceEntry] {
        &self.namespaces
    }

    pub fn ns_state(&mut self) -> &mut ListState {
        &mut self.ns_state
    }

    pub fn pod_state(&mut self) -> &mut ListState {
        &mut self.pod_state
    }

    pub fn cont_state(&mut self) -> &mut ListState {
        &mut self.cont_state
    }

    pub fn selected_namespace(&self) -> Option<&NamespaceEntry> {
        self.namespaces.get(self.ns_state.selected()?)
    }

    pub fn selected_pod(&self) -> Option<&MyPod> {
        self.selected_namespace()?
            .pods
            .get(self.pod_state.selected()?)
    }

    pub fn selected_container(&self) -> Option<&MyContainer> {
        self.selected_pod()?
            .containers
            .get(self.cont_state.selected()?)
    }

    pub fn remote(&self, namespace: &str) -> Option<&RemoteState> {
        self.current_remotes()?.get(namespace)
    }

    pub fn container_status(&self, namespace: &str, container: &MyContainer) -> CommitStatus {
        commit_status(container.image_ref().as_ref(), self.remote(namespace))
    }

    pub fn pod_status(&self, namespace: &str, pod: &MyPod) -> CommitStatus {
        CommitStatus::worst_judged(
            pod.containers
                .iter()
                .map(|container| self.container_status(namespace, container)),
        )
    }

    pub fn namespace_status(&self, entry: &NamespaceEntry) -> CommitStatus {
        if entry.error.is_some() {
            return CommitStatus::Unknown;
        }

        CommitStatus::combined(
            entry
                .pods
                .iter()
                .map(|pod| self.pod_status(&entry.name, pod)),
        )
    }

    /// Picks up whatever background lookups have finished since the last call. Results
    /// are filed under the mode they were requested in, which may no longer be current.
    pub fn drain_lookups(&mut self) {
        loop {
            let (mode, name, state) = match self.lookups.try_recv() {
                Ok((mode, name, Ok(target))) => (mode, name, RemoteState::Ready(target)),
                Ok((mode, name, Err(e))) => (mode, name, RemoteState::Failed(e.details)),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            };

            self.remotes.entry(mode).or_default().insert(name, state);
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }

        let state = match self.focus {
            Focus::Namespaces => &mut self.ns_state,
            Focus::Pods => &mut self.pod_state,
            Focus::Containers => &mut self.cont_state,
        };

        let current = state.selected().unwrap_or(0) as isize;
        let next = current.saturating_add(delta).clamp(0, len as isize - 1) as usize;
        state.select(Some(next));

        match self.focus {
            Focus::Namespaces => self.reset_pods(),
            Focus::Pods => self.reset_containers(),
            Focus::Containers => (),
        }
    }

    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            Focus::Namespaces => Focus::Pods,
            Focus::Pods => Focus::Containers,
            Focus::Containers => Focus::Containers,
        };
    }

    pub fn focus_prev(&mut self) {
        self.focus = match self.focus {
            Focus::Namespaces => Focus::Namespaces,
            Focus::Pods => Focus::Namespaces,
            Focus::Containers => Focus::Pods,
        };
    }

    /// Enter: walks right through the panes, opens the commit on the last one.
    pub fn activate(&mut self) {
        match self.focus {
            Focus::Namespaces | Focus::Pods => self.focus_next(),
            Focus::Containers => self.open_selected(),
        }
    }

    pub fn refresh_selected(&mut self) {
        let Some(entry) = self.selected_namespace() else {
            return;
        };
        let name = entry.name.clone();

        // a second request would spawn another thread and could land out of order,
        // overwriting the newer answer with the older one
        if matches!(self.remote(&name), Some(RemoteState::Loading)) {
            return;
        }

        self.request(&name);
        self.status.clear();
    }

    /// Why the selected namespace cannot be judged, if that is the case — the reason is
    /// otherwise recorded but never shown.
    fn selected_problem(&self) -> Option<String> {
        let entry = self.selected_namespace()?;

        if let Some(error) = &entry.error {
            return Some(format!("{}: {}", entry.name, error));
        }

        match self.remote(&entry.name) {
            Some(RemoteState::Failed(reason)) => Some(format!("{}: {}", entry.name, reason)),
            _ => None,
        }
    }

    fn open_selected(&mut self) {
        let Some(entry) = self.selected_namespace() else {
            return;
        };
        let namespace = entry.name.clone();

        let Some(container) = self.selected_container() else {
            self.status = "No container selected".to_string();
            return;
        };
        let image = container.short_image().to_string();

        let Some(image_ref) = container.image_ref() else {
            self.status = format!("{}: image carries no tag", image);
            return;
        };

        // a release image names a tag, so its commit is only known once tags are fetched
        let hash = match self.remote(&namespace) {
            Some(RemoteState::Ready(target)) => target.commit_of(&image_ref),
            _ => None,
        };

        let Some(hash) = hash else {
            self.status = format!(
                "{}: cannot resolve {} to a commit — try {} mode",
                image,
                image_ref.as_str(),
                self.mode.toggled().label()
            );
            return;
        };

        let Some(repo) = self.repos.get_repo_by_name(&namespace) else {
            self.status = format!("{} is missing in repos.json", namespace);
            return;
        };
        let url = repo.url.clone();

        self.status = match open_with_hash(&url, &hash) {
            Ok(_) => format!("Opened {}", commit_url(&url, &hash)),
            Err(e) => format!("Cannot open browser: {}", e),
        };
    }

    /// Asks for every namespace the current mode has no answer for yet.
    fn request_missing(&mut self) {
        let pending: Vec<String> = self
            .namespaces
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
                spawn_lookup(self.tx.clone(), mode, namespace.to_string(), url);
            }
            None => {
                self.set_remote(
                    namespace,
                    RemoteState::Failed("missing in repos.json".to_string()),
                );
            }
        }
    }

    fn current_remotes(&self) -> Option<&HashMap<String, RemoteState>> {
        self.remotes.get(&self.mode)
    }

    fn set_remote(&mut self, namespace: &str, state: RemoteState) {
        self.remotes
            .entry(self.mode)
            .or_default()
            .insert(namespace.to_string(), state);
    }

    fn list_len(&self) -> usize {
        match self.focus {
            Focus::Namespaces => self.namespaces.len(),
            Focus::Pods => self.selected_namespace().map_or(0, |ns| ns.pods.len()),
            Focus::Containers => self.selected_pod().map_or(0, |p| p.containers.len()),
        }
    }

    fn reset_pods(&mut self) {
        let has_pods = self
            .selected_namespace()
            .is_some_and(|ns| !ns.pods.is_empty());
        self.pod_state.select(has_pods.then_some(0));
        self.reset_containers();
    }

    fn reset_containers(&mut self) {
        let has_containers = self
            .selected_pod()
            .is_some_and(|pod| !pod.containers.is_empty());
        self.cont_state.select(has_containers.then_some(0));
    }
}
