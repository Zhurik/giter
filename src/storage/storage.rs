pub struct Repo {
    name: String,
    url: String,
}

impl Repo {
    fn new(name: String, url: String) -> Repo {
        Repo {
            name,
            url
        }
    }
}

pub trait Storage {
    fn list_repos(&self) -> Vec<Repo>;
}
