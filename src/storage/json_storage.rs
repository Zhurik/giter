use crate::storage::common::{Storage, Repo};
use std::fs;

pub struct JsonStorage {
    repos: Vec<Repo>
}

impl JsonStorage {
    pub fn new(file_path: &'static str) -> JsonStorage {
        let raw_string = match fs::read_to_string(file_path) {
            Ok(f) => f,
            Err(error) => panic!("Problem reading file {:?}: {:?}", file_path, error),
        };

        let repos: Vec<Repo> = match serde_json::from_str(&raw_string) {
            Ok(data) => data,
            Err(error) => panic!("Problem serializing string '{:?}' : {:?}", raw_string, error),
        };

        JsonStorage {
            repos
        }
    }
}

impl Storage for JsonStorage {
    fn list_repos(&self) -> &Vec<Repo> {
        &self.repos
    }
}
