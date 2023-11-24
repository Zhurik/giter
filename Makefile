format:
	cargo fmt

release: format
	cargo build --release && cp ./target/release/giter ~/
