FROM rust:1.90-bookworm
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release --locked
ENTRYPOINT ["/app/target/release/generate"]
