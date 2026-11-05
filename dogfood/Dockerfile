# Build context: the repository root (the app depends on the crate at `..`).
FROM rust:1.99.1-slim-trixie AS builder
WORKDIR /src
COPY Cargo.toml ./
COPY src ./src
COPY frontend/dist ./frontend/dist
COPY dogfood ./dogfood
WORKDIR /src/dogfood
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/dogfood/target \
    cargo build --release --locked --bin dogfood-cli \
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
