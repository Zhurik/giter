use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct MsgError {
    pub details: String,
}

impl MsgError {
    pub fn new(msg: &str) -> MsgError {
        MsgError {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for MsgError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for MsgError {
    fn description(&self) -> &str {
        &self.details
    }
}
