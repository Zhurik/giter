use ::open;

pub fn open_with_hash(url: &String, hash: &String) -> Result<(), std::io::Error> {
    open::that(format!("{}/-/commit/{}", url, hash))
}
