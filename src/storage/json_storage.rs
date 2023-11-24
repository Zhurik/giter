use crate::storage::common::{Repo, Storage};
use std::{error::Error, fs};

pub struct JsonStorage {
    repos: Vec<Repo>,
}

impl JsonStorage {
    pub fn new(file_path: &'static str) -> Result<JsonStorage, Box<dyn Error>> {
        let raw_string = match fs::read_to_string(file_path) {
            Ok(f) => f,
            Err(e) => return Err(Box::new(e)),
        };

        let repos: Vec<Repo> = match serde_json::from_str(&raw_string) {
            Ok(data) => data,
            Err(e) => return Err(Box::new(e)),
        };

        Ok(JsonStorage { repos })
    }
}

impl Storage for JsonStorage {
    fn list_repos(&self) -> &Vec<Repo> {
        &self.repos
    }
}
