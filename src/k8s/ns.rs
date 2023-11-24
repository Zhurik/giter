use std::{io::Error, process::Command};

pub fn get_current_namespace() -> Result<String, Error> {
    let raw_ns = match Command::new("kubens").arg("-c").output() {
        Ok(x) => x,
        Err(e) => return Err(e),
    };

    let mut ns = match String::from_utf8(raw_ns.stdout) {
        Ok(x) => x,
        Err(_) => return Err(Error::other("Incorrect output from kubens")),
    };

    ns.pop();

    Ok(ns)
}
