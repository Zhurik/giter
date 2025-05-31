FROM lukemathwalker/cargo-chef:latest-rust-1@sha256:e20143b49d4be9fb770864354ea64ffedd912c6e145465004a3e215204b22e17 AS chef
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
FROM debian:bookworm-slim@sha256:90522eeb7e5923ee2b871c639059537b30521272f10ca86fdbbbb2b75a8c40cd AS runtime
WORKDIR /giter
COPY --from=builder /giter/target/release/giter /usr/local/bin
ENTRYPOINT ["/usr/local/bin/giter"]
