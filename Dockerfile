FROM lukemathwalker/cargo-chef:latest-rust-1@sha256:420b7375dc4059a85f1c365f23f96b8f561c2861e365645b413c531b206c528a AS chef
WORKDIR /giter

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /giter/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build giterlication
COPY . .
RUN cargo build --release --bin giter

# We do not need the Rust toolchain to run the binary!
FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS runtime
WORKDIR /giter
COPY --from=builder /giter/target/release/giter /usr/local/bin
ENTRYPOINT ["/usr/local/bin/giter"]
