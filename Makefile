.PHONY: format lint test check release

format:
	cargo fmt

lint:
	cargo fmt --check
	cargo clippy --all-targets --locked -- -D warnings

test:
	cargo test --all --locked

# Everything CI runs on a pull request, minus the cross builds.
check: lint test
	RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --locked
	pre-commit run --all-files

release: format
	cargo build --release && cp ./target/release/giter ~/
