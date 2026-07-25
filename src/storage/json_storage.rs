use crate::storage::common::{Repo, Storage};
use std::{error::Error, fs};

pub struct JsonStorage {
    repos: Vec<Repo>,
}

impl JsonStorage {
    pub fn new(file_path: &str) -> Result<JsonStorage, Box<dyn Error>> {
        let raw_string = fs::read_to_string(file_path)?;
        let repos: Vec<Repo> = serde_json::from_str(&raw_string)?;

        Ok(JsonStorage { repos })
    }

    pub fn get_repo_by_name(&self, name: &str) -> Option<&Repo> {
        self.repos.iter().find(|r| r.name == name)
    }
}

impl Storage for JsonStorage {
    fn list_repos(&self) -> &Vec<Repo> {
        &self.repos
    }
}
