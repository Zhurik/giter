use crate::storage::storage::{Storage, Repo};
use std::fs::File;

struct JsonStorage {
    file_path: &'static str
}

impl JsonStorage {
    fn new(file_path: &'static str) -> JsonStorage {
        JsonStorage {
            file_path
        }
    }
}

impl Storage for JsonStorage {
    fn list_repos(&self) -> Vec<Repo> {
        let mut file = match File::open(self.file_path) {
            Ok(f) => f,
            Err(error) => panic!("Problem opening the file: {:?}", error),
        };

        let mut data =

        vec![]
    }
}
