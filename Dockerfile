FROM lukemathwalker/cargo-chef:latest-rust-1@sha256:b9e64e59c4282a64f80ea427761a26be5462f33cbc6e1797053222f3d26200e0 AS chef
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
FROM debian:bookworm-slim@sha256:4b50eb66f977b4062683ff434ef18ac191da862dbe966961bc11990cf5791a8d AS runtime
WORKDIR /giter
COPY --from=builder /giter/target/release/giter /usr/local/bin
ENTRYPOINT ["/usr/local/bin/giter"]
