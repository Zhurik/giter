use std::process::Command;

pub fn get_current_namespace() -> String {
    let raw_ns = Command::new("kubens")
        .arg("-c")
        .output()
        .unwrap();

    let mut ns = String::from_utf8(raw_ns.stdout).unwrap();

    ns.pop();

    ns
}
