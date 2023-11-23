use serde::Deserialize;

#[derive(Deserialize, Clone)]

pub struct Repo {
    pub name: String,
    pub url: String,
}

pub trait Storage {
    fn list_repos(&self) -> &Vec<Repo>;
}
