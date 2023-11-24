use crate::errors::MsgError;
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

    pub fn commit_hash(&self) -> Result<String, MsgError> {
        let hash = match self.image.split(":").nth(1) {
            Some(tag) => match tag.split("-").nth(0) {
                Some(x) => x,
                None => return Err(MsgError::new("Wrong tag format")),
            },
            None => return Err(MsgError::new("Missing tag in image")),
        };

        Ok(hash.to_string())
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

    pub async fn get_pods_by_ns(namespace: String) -> Result<Vec<MyPod>, MsgError> {
        let client = match Client::try_default().await {
            Ok(x) => x,
            Err(_) => return Err(MsgError::new("Cannot initialize client")),
        };

        let pods_api: Api<Pod> = Api::namespaced(client, &namespace);

        let pods = match pods_api.list(&ListParams::default()).await {
            Ok(x) => x,
            Err(_) => return Err(MsgError::new("Cannot list pods")),
        };

        let mut my_pods: Vec<MyPod> = vec![];

        for pod in pods {
            let mut containers: Vec<MyContainer> = vec![];

            let pod_name = pod.metadata.name.unwrap_or_default();

            let pod_ns = pod.metadata.namespace.unwrap_or_default();

            let pod_containers = match pod.spec {
                Some(spec) => spec.containers,
                None => return Err(MsgError::new("Missing spec")),
            };

            for cont in pod_containers {
                let cont_image = cont.image.unwrap_or_default();
                let cont_name = cont.name;

                containers.push(MyContainer::new(cont_name, cont_image));
            }

            my_pods.push(MyPod::new(pod_name, pod_ns, containers))
        }

        Ok(my_pods)
    }
}
