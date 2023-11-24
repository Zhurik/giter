use crate::errors::MsgError;
use kube::config::Kubeconfig;

pub fn get_current_namespace() -> Result<String, MsgError> {
    let config = match Kubeconfig::from_env() {
        Ok(config_option) => match config_option {
            Some(x) => x,
            None => return Err(MsgError::new("Couldn't unpack config")),
        },
        Err(_) => return Err(MsgError::new("Couldn't parse config")),
    };

    let current_context = match config.current_context {
        Some(x) => x,
        None => return Err(MsgError::new("Not current context specified")),
    };

    let context = match config
        .contexts
        .iter()
        .filter(|a| a.name == current_context)
        .next()
    {
        Some(named) => match named.context.to_owned() {
            Some(x) => x,
            None => return Err(MsgError::new("Couldn't open proper context")),
        },
        None => return Err(MsgError::new("Couldn't find proper context")),
    };

    match context.namespace {
        Some(x) => Ok(x),
        None => return Err(MsgError::new("Current namespace is not set")),
    }
}
