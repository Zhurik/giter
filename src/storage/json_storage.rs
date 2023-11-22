use crate::storage::common::{Storage, Repo};
use std::fs;

pub struct JsonStorage {
    file_path: &'static str
}

impl JsonStorage {
    pub fn new(file_path: &'static str) -> JsonStorage {
        JsonStorage {
            file_path
        }
    }
}

impl Storage for JsonStorage {
    fn list_repos(&self) -> Vec<Repo> {
        let raw_string = match fs::read_to_string(self.file_path) {
            Ok(f) => f,
            Err(error) => panic!("Problem reading file {:?}: {:?}", self.file_path, error),
        };

        let repos: Vec<Repo> = match serde_json::from_str(&raw_string) {
            Ok(data) => data,
            Err(error) => panic!("Problem serializing string '{:?}' : {:?}", raw_string, error),
        };

        repos
    }
}
