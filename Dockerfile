FROM rust:1.100.0-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN cargo build --locked --release --bin ai-security-review

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/ai-security-review /usr/local/bin/ai-security-review
USER 65532:65532
WORKDIR /work
ENTRYPOINT ["/usr/local/bin/ai-security-review"]
