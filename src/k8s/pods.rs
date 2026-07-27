use crate::errors::MsgError;
use crate::git::remote::ImageRef;
use crate::text::short_duration;
use k8s_openapi::api::core::v1::{Pod, PodStatus};
use k8s_openapi::chrono::{DateTime, Utc};
use kube::{
    api::{Api, ListParams},
    Client,
};

const UNKNOWN_PHASE: &str = "Unknown";
const TERMINATING: &str = "Terminating";
const RUNNING: &str = "Running";

/// Pod states nobody has to look at: it runs, it is on its way in or out, or it is a job
/// pod that has done its work — normal for anything a CronJob leaves behind.
const CALM: [&str; 7] = [
    RUNNING,
    "Completed",
    "Succeeded",
    "Pending",
    "ContainerCreating",
    "PodInitializing",
    TERMINATING,
];

/// Phases a pod reaches once it has stopped running for good. A pod of a Deployment never
/// gets there — its restart policy is `Always` — so this is what a Job leaves behind.
const FINISHED: [&str; 2] = ["Succeeded", "Failed"];

/// Whether a pod's state is one to leave alone, as opposed to one worth a colour.
pub fn is_calm(status: &str) -> bool {
    CALM.contains(&status)
}

pub struct MyContainer {
    pub name: String,
    pub image: String,
}

impl MyContainer {
    fn new(name: String, image: String) -> MyContainer {
        MyContainer { name, image }
    }

    /// Image without the registry and project path, e.g. `orchard-gateway:3ac91b7e-8103771`.
    pub fn short_image(&self) -> &str {
        self.image.rsplit('/').next().unwrap_or(&self.image)
    }

    /// Tag the image is pinned to, e.g. `3ac91b7e-8103771`.
    ///
    /// `None` for an image pinned by digest, where the part after the colon is the digest
    /// rather than a tag.
    pub fn tag(&self) -> Option<&str> {
        let image = self.short_image();

        match image.contains('@') {
            true => None,
            false => Some(image.rsplit_once(':')?.1),
        }
    }

    /// What the image was built from, as far as the tag tells.
    pub fn image_ref(&self) -> Option<ImageRef> {
        ImageRef::from_image_tag(self.tag()?)
    }
}

pub struct MyPod {
    pub name: String,
    pub namespace: String,
    pub containers: Vec<MyContainer>,
    /// What `kubectl` shows in its STATUS column: a waiting container's reason if there is
    /// one, otherwise the phase. A pod stuck in `CrashLoopBackOff` next to a lagging
    /// commit explains a rollout that never finished.
    pub status: String,
    pub restarts: i32,
    pub ready: usize,
    pub created: Option<DateTime<Utc>>,
    /// The pod has run its course and is not deploying anything any more. What a CronJob
    /// leaves behind ran the code that was current at the time, and says nothing about what
    /// is deployed now.
    pub finished: bool,
}

impl MyPod {
    fn from_pod(pod: Pod, namespace: &str) -> MyPod {
        let status = status_text(&pod);
        let restarts = restart_count(pod.status.as_ref());
        let ready = ready_count(pod.status.as_ref());
        let finished = has_finished(pod.status.as_ref());

        let containers = pod
            .spec
            .map(|spec| spec.containers)
            .unwrap_or_default()
            .into_iter()
            .map(|cont| MyContainer::new(cont.name, cont.image.unwrap_or_default()))
            .collect();

        MyPod {
            name: pod.metadata.name.unwrap_or_default(),
            namespace: pod
                .metadata
                .namespace
                .unwrap_or_else(|| namespace.to_string()),
            containers,
            status,
            restarts,
            ready,
            created: pod.metadata.creation_timestamp.map(|time| time.0),
            finished,
        }
    }

    /// How long the pod has been around, in the compact shape `kubectl` uses.
    pub fn age(&self, now: DateTime<Utc>) -> Option<String> {
        let created = self.created?;

        Some(short_duration((now - created).num_seconds()))
    }

    /// Ready containers over total, e.g. `2/2`.
    pub fn readiness(&self) -> String {
        format!("{}/{}", self.ready, self.containers.len())
    }

    pub async fn get_pods_by_ns(client: &Client, namespace: &str) -> Result<Vec<MyPod>, MsgError> {
        let pods_api: Api<Pod> = Api::namespaced(client.clone(), namespace);

        let pods = match pods_api.list(&ListParams::default()).await {
            Ok(x) => x,
            Err(e) => return Err(MsgError::new(&list_error(e))),
        };

        Ok(pods
            .into_iter()
            .map(|pod| MyPod::from_pod(pod, namespace))
            .collect())
    }
}

/// Keeps only the part of a kube error worth a table cell: the API reason such as
/// `forbidden`, not the whole request context.
fn list_error(error: kube::Error) -> String {
    match error {
        kube::Error::Api(response) if !response.reason.is_empty() => response.reason.to_lowercase(),
        kube::Error::Api(response) => response.message,
        other => other.to_string(),
    }
}

fn status_text(pod: &Pod) -> String {
    if pod.metadata.deletion_timestamp.is_some() {
        return TERMINATING.to_string();
    }

    let status = pod.status.as_ref();

    waiting_reason(status)
        .or_else(|| terminated_reason(status))
        .or_else(|| status.and_then(|s| s.phase.clone()))
        .unwrap_or_else(|| UNKNOWN_PHASE.to_string())
}

fn waiting_reason(status: Option<&PodStatus>) -> Option<String> {
    container_statuses(status)?
        .iter()
        .find_map(|c| c.state.as_ref()?.waiting.as_ref()?.reason.clone())
}

fn terminated_reason(status: Option<&PodStatus>) -> Option<String> {
    container_statuses(status)?
        .iter()
        .find_map(|c| c.state.as_ref()?.terminated.as_ref()?.reason.clone())
}

fn container_statuses(
    status: Option<&PodStatus>,
) -> Option<&Vec<k8s_openapi::api::core::v1::ContainerStatus>> {
    status?.container_statuses.as_ref()
}

fn has_finished(status: Option<&PodStatus>) -> bool {
    status
        .and_then(|s| s.phase.as_deref())
        .is_some_and(|phase| FINISHED.contains(&phase))
}

fn restart_count(status: Option<&PodStatus>) -> i32 {
    container_statuses(status)
        .map(|list| list.iter().map(|c| c.restart_count).sum())
        .unwrap_or_default()
}

fn ready_count(status: Option<&PodStatus>) -> usize {
    container_statuses(status)
        .map(|list| list.iter().filter(|c| c.ready).count())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateTerminated, ContainerStateWaiting, ContainerStatus, PodSpec,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use k8s_openapi::chrono::TimeZone;
    use speculoos::prelude::*;

    fn container(image: &str) -> MyContainer {
        MyContainer::new("app".to_string(), image.to_string())
    }

    fn waiting(reason: &str) -> ContainerStatus {
        ContainerStatus {
            state: Some(ContainerState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some(reason.to_string()),
                    ..ContainerStateWaiting::default()
                }),
                ..ContainerState::default()
            }),
            ..ContainerStatus::default()
        }
    }

    fn terminated(reason: &str) -> ContainerStatus {
        ContainerStatus {
            state: Some(ContainerState {
                terminated: Some(ContainerStateTerminated {
                    reason: Some(reason.to_string()),
                    ..ContainerStateTerminated::default()
                }),
                ..ContainerState::default()
            }),
            ..ContainerStatus::default()
        }
    }

    fn restarted(count: i32, is_ready: bool) -> ContainerStatus {
        ContainerStatus {
            restart_count: count,
            ready: is_ready,
            ..ContainerStatus::default()
        }
    }

    fn pod(phase: &str, statuses: Vec<ContainerStatus>) -> Pod {
        Pod {
            status: Some(PodStatus {
                phase: Some(phase.to_string()),
                container_statuses: Some(statuses),
                ..PodStatus::default()
            }),
            ..Pod::default()
        }
    }

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
    }

    #[test]
    fn branch_build_carries_a_commit() {
        let commit = "3ac91b7e";
        let cont = container(&format!(
            "registry.example/orchard-gateway:{}-8103771",
            commit
        ));

        assert_that!(cont.image_ref()).is_equal_to(Some(ImageRef::Commit(commit.to_string())));
    }

    #[test]
    fn release_build_carries_a_tag() {
        let release = "v3.7.1";
        let cont = container(&format!(
            "registry.example/orchard-ledger-api:{}-8203114",
            release
        ));

        assert_that!(cont.image_ref()).is_equal_to(Some(ImageRef::Tag(release.to_string())));
    }

    #[test]
    fn image_without_a_tag_has_no_reference() {
        let cont = container("registry.example/products/driftwood");

        assert_that!(cont.image_ref()).is_none();
    }

    #[test]
    fn registry_port_is_not_read_as_a_tag() {
        let cont = container("registry.example:5000/nimbus/cache-component");

        asserting!("the port belongs to the registry host")
            .that(&cont.tag())
            .is_none();
    }

    #[test]
    fn image_pinned_by_digest_has_no_tag() {
        let digest = "sha256:0b7e19f4a3c2d85610fe4477bb90d1c6e5382af0947dc61b8e2f5a3049cd7f12";
        let cont = container(&format!(
            "registry.example/nimbus-index-component@{}",
            digest
        ));

        asserting!("a digest is not a tag")
            .that(&cont.tag())
            .is_none();
    }

    #[test]
    fn tag_is_the_part_after_the_colon() {
        let tag = "9d02f4ab-7710455";
        let cont = container(&format!("registry.example/tulip-mailer:{}", tag));

        assert_that!(cont.tag()).is_some().is_equal_to(tag);
    }

    #[test]
    fn short_image_drops_the_registry() {
        let image = "pigeon-post:5f3c17d9-7710462";
        let cont = container(&format!("registry.example/products/{}", image));

        assert_that!(cont.short_image()).is_equal_to(image);
    }

    #[test]
    fn healthy_pod_reports_its_phase() {
        let ready = MyPod::from_pod(pod(RUNNING, vec![restarted(0, true)]), "orchard-gateway");

        assert_that!(ready.status).is_equal_to(RUNNING.to_string());
    }

    #[test]
    fn waiting_container_names_what_holds_the_pod_back() {
        let crashing = "CrashLoopBackOff";
        let stuck = MyPod::from_pod(pod(RUNNING, vec![waiting(crashing)]), "driftwood");

        asserting!("the phase would still say Running while nothing works")
            .that(&stuck.status)
            .is_equal_to(crashing.to_string());
    }

    #[test]
    fn finished_container_reports_why_it_stopped() {
        let finished =
            MyPod::from_pod(pod("Succeeded", vec![terminated("Completed")]), "driftwood");

        assert_that!(finished.status).is_equal_to("Completed".to_string());
    }

    #[test]
    fn pod_being_deleted_is_terminating() {
        let mut leaving = pod(RUNNING, vec![restarted(0, true)]);
        leaving.metadata.deletion_timestamp = Some(Time(at(2026, 7, 20)));

        assert_that!(MyPod::from_pod(leaving, "driftwood").status)
            .is_equal_to(TERMINATING.to_string());
    }

    #[test]
    fn pod_without_any_status_is_unknown() {
        let bare = MyPod::from_pod(Pod::default(), "nimbus-index-component");

        assert_that!(bare.status).is_equal_to(UNKNOWN_PHASE.to_string());
    }

    #[test]
    fn finished_job_pod_needs_no_attention() {
        asserting!("a pod a CronJob left behind has done nothing wrong")
            .that(&is_calm("Completed"))
            .is_true();
    }

    #[test]
    fn starting_pod_needs_no_attention() {
        assert_that!(is_calm("ContainerCreating")).is_true();
    }

    #[test]
    fn crash_looping_pod_is_worth_a_colour() {
        assert_that!(is_calm("CrashLoopBackOff")).is_false();
    }

    #[test]
    fn pod_that_could_not_pull_its_image_is_worth_a_colour() {
        assert_that!(is_calm("ImagePullBackOff")).is_false();
    }

    #[test]
    fn job_pod_that_ran_its_course_is_finished() {
        let done = MyPod::from_pod(
            pod("Succeeded", vec![terminated("Completed")]),
            "tulip-mailer",
        );

        asserting!("it deploys nothing any more, whatever it once ran")
            .that(&done.finished)
            .is_true();
    }

    #[test]
    fn serving_pod_is_not_finished() {
        let serving = MyPod::from_pod(pod(RUNNING, vec![restarted(0, true)]), "orchard-gateway");

        assert_that!(serving.finished).is_false();
    }

    #[test]
    fn crashing_pod_is_not_finished() {
        let crashing =
            MyPod::from_pod(pod(RUNNING, vec![waiting("CrashLoopBackOff")]), "driftwood");

        asserting!("it is still trying, and it is still what the namespace runs")
            .that(&crashing.finished)
            .is_false();
    }

    #[test]
    fn restarts_are_summed_over_the_containers() {
        let flapping = MyPod::from_pod(
            pod(RUNNING, vec![restarted(5, true), restarted(2, true)]),
            "orchard-ledger-api",
        );

        assert_that!(flapping.restarts).is_equal_to(7);
    }

    #[test]
    fn readiness_counts_only_the_ready_containers() {
        let mut half = pod(RUNNING, vec![restarted(0, true), restarted(3, false)]);
        half.spec = Some(PodSpec {
            containers: vec![Default::default(), Default::default()],
            ..PodSpec::default()
        });

        assert_that!(MyPod::from_pod(half, "orchard-ledger-api").readiness())
            .is_equal_to("1/2".to_string());
    }

    #[test]
    fn pod_without_a_creation_time_has_no_age() {
        let bare = MyPod::from_pod(Pod::default(), "tulip-mailer");

        assert_that!(bare.age(at(2026, 7, 25))).is_none();
    }

    #[test]
    fn age_is_counted_from_the_creation_time() {
        let mut born = pod(RUNNING, vec![]);
        born.metadata.creation_timestamp = Some(Time(at(2026, 7, 19)));

        assert_that!(MyPod::from_pod(born, "tulip-mailer").age(at(2026, 7, 25)))
            .is_some()
            .is_equal_to("6d".to_string());
    }
}
