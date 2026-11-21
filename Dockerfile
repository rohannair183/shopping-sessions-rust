FROM rust:1.90-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release --locked && rm -rf src
COPY src/ src/
RUN touch src/main.rs && cargo build --release --locked

FROM debian:bookworm-slim@sha256:c67e92c8099ba22853ab4cfda7cfb494d253d6012ca82dee98ba8976772af018
COPY --from=build /app/target/release/shopping-sessions /usr/local/bin/shopping-sessions
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/shopping-sessions"]
