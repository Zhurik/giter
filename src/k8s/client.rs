use crate::errors::MsgError;
use kube::Client;

/// Single client shared by every namespace lookup — cloning it is cheap,
/// building it parses the kubeconfig and sets up TLS.
pub async fn try_default() -> Result<Client, MsgError> {
    match Client::try_default().await {
        Ok(client) => Ok(client),
        Err(_) => Err(MsgError::new("Cannot initialize client")),
    }
}
