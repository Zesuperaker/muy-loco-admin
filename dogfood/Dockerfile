# Build context: the repository root (the app depends on the crate at `..`).
FROM rust:1.99.1-slim-trixie AS builder
WORKDIR /src
# Dependencies first, built against empty sources: this layer is reused (locally and from CI's
# layer cache) until a manifest or Cargo.lock changes, so a source change only rebuilds the three
# local crates. Cache mounts would not help CI, whose cache does not keep them. A new workspace
# member or binary needs its manifest and stub here.
COPY Cargo.toml ./
COPY dogfood/Cargo.toml dogfood/Cargo.lock ./dogfood/
COPY dogfood/migration/Cargo.toml ./dogfood/migration/
RUN mkdir -p src dogfood/src/bin dogfood/migration/src \
    && touch src/lib.rs dogfood/src/lib.rs dogfood/migration/src/lib.rs \
    && echo 'fn main() {}' > dogfood/src/bin/main.rs \
    && cargo build --manifest-path dogfood/Cargo.toml --release --locked --bin dogfood-cli \
    && cargo clean --manifest-path dogfood/Cargo.toml --release --locked \
        -p muy-loco-admin -p dogfood -p migration \
    && rm -r src dogfood/src dogfood/migration/src
COPY src ./src
COPY frontend/dist ./frontend/dist
COPY dogfood ./dogfood
WORKDIR /src/dogfood
RUN cargo build --release --locked --bin dogfood-cli \
    && cp target/release/dogfood-cli /usr/local/bin/dogfood-cli

FROM debian:trixie-20261027-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /app dogfood
WORKDIR /app
COPY --from=builder /usr/local/bin/dogfood-cli /usr/local/bin/dogfood-cli
COPY dogfood/config ./config
COPY dogfood/assets ./assets
COPY dogfood/src/fixtures ./src/fixtures
COPY --chmod=755 dogfood/docker/entrypoint.sh /usr/local/bin/entrypoint.sh
# A Windows checkout may have converted the script to CRLF.
RUN sed -i 's/\r$//' /usr/local/bin/entrypoint.sh
USER dogfood
ENV LOCO_ENV=production
EXPOSE 5150
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
