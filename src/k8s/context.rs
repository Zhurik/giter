use kube::config::Kubeconfig;

/// Environment a cluster belongs to. Drives the colour of the whole header, so that
/// production cannot be mistaken for staging out of the corner of an eye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Prod,
    Stage,
    Dev,
}

impl Env {
    /// Reads the environment out of a kubeconfig context name, e.g. `orchard-prod` → `Prod`.
    /// `None` when the name says nothing, in which case no badge is shown at all.
    pub fn from_context(context: &str) -> Option<Env> {
        let name = context.to_lowercase();

        // clipped so that `production`, `staging` and `develop` are matched as well
        [(Env::Prod, "prod"), (Env::Stage, "stag"), (Env::Dev, "dev")]
            .into_iter()
            .find(|(_, needle)| name.contains(needle))
            .map(|(env, _)| env)
    }

    pub fn label(self) -> &'static str {
        match self {
            Env::Prod => "PROD",
            Env::Stage => "STAGE",
            Env::Dev => "DEV",
        }
    }
}

/// Which cluster the numbers on screen are about.
#[derive(Debug, Clone, Default)]
pub struct Cluster {
    pub context: Option<String>,
    pub env: Option<Env>,
}

/// Current context of the kubeconfig. Everything is optional: in-cluster there is no
/// kubeconfig to read, and the tool still works — it just cannot name the cluster.
pub fn current() -> Cluster {
    let context = Kubeconfig::read().ok().and_then(|c| c.current_context);
    let env = context.as_deref().and_then(Env::from_context);

    Cluster { context, env }
}

#[cfg(test)]
mod tests {
    use super::*;
    use speculoos::prelude::*;

    #[test]
    fn production_context_is_recognised() {
        assert_that!(Env::from_context("orchard-prod")).is_equal_to(Some(Env::Prod));
    }

    #[test]
    fn staging_context_is_recognised() {
        assert_that!(Env::from_context("nimbus-staging-1")).is_equal_to(Some(Env::Stage));
    }

    #[test]
    fn development_context_is_recognised() {
        assert_that!(Env::from_context("dev-cluster")).is_equal_to(Some(Env::Dev));
    }

    #[test]
    fn context_name_is_matched_ignoring_case() {
        assert_that!(Env::from_context("ORCHARD-PROD")).is_equal_to(Some(Env::Prod));
    }

    #[test]
    fn production_wins_when_several_words_match() {
        asserting!("a context deploying prod from a dev jump host is still prod")
            .that(&Env::from_context("jump-dev-prod-1"))
            .is_equal_to(Some(Env::Prod));
    }

    #[test]
    fn context_naming_no_environment_gets_no_badge() {
        assert_that!(Env::from_context("minikube")).is_none();
    }
}
