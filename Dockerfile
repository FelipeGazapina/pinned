# The hub, as Railway runs it: Postgres comes in DATABASE_URL, the sealing key lives on the volume at /data.
FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends python3-pip && rm -rf /var/lib/apt/lists/* \
    && pip3 install --break-system-packages ziglang \
    && cargo install --locked cargo-zigbuild \
    && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
WORKDIR /src
COPY . .
RUN cargo zigbuild --release -p pinned-server --target x86_64-unknown-linux-musl --target aarch64-unknown-linux-musl \
    && mkdir /downloads \
    && cp target/x86_64-unknown-linux-musl/release/pinned-server /downloads/pinned-server-x86_64-unknown-linux-musl \
    && cp target/aarch64-unknown-linux-musl/release/pinned-server /downloads/pinned-server-aarch64-unknown-linux-musl

FROM alpine:3.20
RUN apk add --no-cache ca-certificates
COPY --from=build /downloads /app/downloads
COPY --from=build /downloads/pinned-server-x86_64-unknown-linux-musl /usr/local/bin/pinned-server
COPY ui /app/ui
CMD exec pinned-server hub --db "$DATABASE_URL" --listen "0.0.0.0:${PORT:-8787}" --ui /app/ui --downloads /app/downloads \
    --public-url "https://$RAILWAY_PUBLIC_DOMAIN" --key-file /data/hub.key
