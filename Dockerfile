ARG APP=aricanduva

FROM lukemathwalker/cargo-chef:latest-rust-1.93 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder 
ARG APP
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin ${APP}

FROM debian:stable-slim AS runtime
ARG APP
LABEL org.opencontainers.image.title="${APP}"
LABEL org.opencontainers.image.description="A Rust web server providing S3-like endpoints with SQLite storage, proxying to IPFS for content."
WORKDIR /app
COPY --from=builder /app/target/release/${APP} /usr/local/bin
CMD ["/usr/local/bin/aricanduva"]
