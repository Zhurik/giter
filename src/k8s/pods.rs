use crate::errors::MsgError;
use crate::git::remote::ImageRef;
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams},
    Client,
};

pub struct MyContainer {
    pub name: String,
    pub image: String,
}

impl MyContainer {
    fn new(name: String, image: String) -> MyContainer {
        MyContainer { name, image }
    }

    /// Image without the registry and project path, e.g. `usage-api:24e8b66c-5409306`.
    pub fn short_image(&self) -> &str {
        self.image.rsplit('/').next().unwrap_or(&self.image)
    }

    /// Tag the image is pinned to, e.g. `24e8b66c-5409306`.
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
}

impl MyPod {
    fn new(name: String, namespace: String, containers: Vec<MyContainer>) -> MyPod {
        MyPod {
            name,
            namespace,
            containers,
        }
    }

    pub async fn get_pods_by_ns(client: &Client, namespace: &str) -> Result<Vec<MyPod>, MsgError> {
        let pods_api: Api<Pod> = Api::namespaced(client.clone(), namespace);

        let pods = match pods_api.list(&ListParams::default()).await {
            Ok(x) => x,
            Err(_) => return Err(MsgError::new("Cannot list pods")),
        };

        let mut my_pods: Vec<MyPod> = vec![];

        for pod in pods {
            let pod_name = pod.metadata.name.unwrap_or_default();

            let pod_ns = pod
                .metadata
                .namespace
                .unwrap_or_else(|| namespace.to_string());

            let pod_containers = pod.spec.map(|spec| spec.containers).unwrap_or_default();

            let containers: Vec<MyContainer> = pod_containers
                .into_iter()
                .map(|cont| MyContainer::new(cont.name, cont.image.unwrap_or_default()))
                .collect();

            my_pods.push(MyPod::new(pod_name, pod_ns, containers))
        }

        Ok(my_pods)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speculoos::prelude::*;

    fn container(image: &str) -> MyContainer {
        MyContainer::new("app".to_string(), image.to_string())
    }

    #[test]
    fn branch_build_carries_a_commit() {
        let commit = "8745ffc5";
        let cont = container(&format!("registry.example/admin-api:{}-5392582", commit));

        assert_that!(cont.image_ref()).is_equal_to(Some(ImageRef::Commit(commit.to_string())));
    }

    #[test]
    fn release_build_carries_a_tag() {
        let release = "v0.44.3";
        let cont = container(&format!("registry.example/public-api:{}-5325504", release));

        assert_that!(cont.image_ref()).is_equal_to(Some(ImageRef::Tag(release.to_string())));
    }

    #[test]
    fn image_without_a_tag_has_no_reference() {
        let cont = container("registry.example/products/keppel");

        assert_that!(cont.image_ref()).is_none();
    }

    #[test]
    fn registry_port_is_not_read_as_a_tag() {
        let cont = container("registry.example:5000/mellivora/npm-component");

        asserting!("the port belongs to the registry host")
            .that(&cont.tag())
            .is_none();
    }

    #[test]
    fn image_pinned_by_digest_has_no_tag() {
        let digest = "sha256:2424c1850714a4d94666ec928e24d86de958646737b1d113f5b2207be44d37d8";
        let cont = container(&format!("registry.example/debian-component@{}", digest));

        asserting!("a digest is not a tag")
            .that(&cont.tag())
            .is_none();
    }

    #[test]
    fn tag_is_the_part_after_the_colon() {
        let tag = "1885c684-5432308";
        let cont = container(&format!("registry.example/rpm-component:{}", tag));

        assert_that!(cont.tag()).is_some().is_equal_to(tag);
    }

    #[test]
    fn short_image_drops_the_registry() {
        let image = "pypi-component:99f6dabd-5432278";
        let cont = container(&format!("registry.example/products/{}", image));

        assert_that!(cont.short_image()).is_equal_to(image);
    }
}
