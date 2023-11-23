use std::process::{Command, Stdio};

pub fn get_pods_image_hashes() -> Vec<String> {
    let pods_child = Command::new("kubectl")
        .arg("describe")
        .arg("pods")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let grep_res = Command::new("grep")
        .arg("Image:")
        .stdin(Stdio::from(pods_child.stdout.unwrap()))
        .output()
        .unwrap();

    let raw_string = String::from_utf8(grep_res.stdout).unwrap();

    let parts = raw_string.split("\n");

    let mut raw_hashes: Vec<String> = parts
        .map(|s| {
            s.split(":")
                .nth(2)
                .unwrap_or_default()
                .split("-")
                .nth(0)
                .unwrap_or_default()
        })
        .map(|s| s.to_string())
        .collect();

    raw_hashes.dedup();

    raw_hashes
}
