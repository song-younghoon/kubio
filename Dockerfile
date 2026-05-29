FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p kubio-cli

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/kubio /usr/local/bin/kubio
EXPOSE 8080 9900
ENTRYPOINT ["kubio"]
