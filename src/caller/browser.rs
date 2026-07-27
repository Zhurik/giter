use ::open;

pub fn commit_url(url: &str, hash: &str) -> String {
    format!("{}/-/commit/{}", url, hash)
}

pub fn open_with_hash(url: &str, hash: &str) -> Result<(), std::io::Error> {
    open::that(commit_url(url, hash))
}
