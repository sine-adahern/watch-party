# ---- build the WASM frontend ----
FROM rust:1-bookworm AS web
RUN rustup target add wasm32-unknown-unknown && cargo install trunk
WORKDIR /app
COPY . .
RUN cd web && trunk build --release

# ---- build the server ----
FROM rust:1-bookworm AS server
WORKDIR /app
COPY . .
RUN cargo build -p server --release

# ---- runtime ----
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ffmpeg ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server /app/target/release/server /usr/local/bin/server
COPY --from=web    /app/web/dist              /app/web/dist
ENV MEDIA_DIR=/data
VOLUME ["/data"]
EXPOSE 3000
CMD ["server"]
