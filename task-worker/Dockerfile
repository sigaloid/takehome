FROM rust:1-alpine AS builder

RUN apk add --no-cache build-base \
    && rustup target add x86_64-unknown-linux-musl

WORKDIR /app

# Cache dependency compilation for the musl target.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release --target x86_64-unknown-linux-musl \
    && rm -rf src

# Build the actual application for the musl target.
COPY src ./src
RUN cargo build --release --target x86_64-unknown-linux-musl \
    && strip target/x86_64-unknown-linux-musl/release/task-worker

FROM scratch AS runtime

COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/task-worker /task-worker

ENV RUST_LOG=info

ENTRYPOINT ["/task-worker"]
