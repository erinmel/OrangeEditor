FROM rust:1.99.1-slim-bookworm AS build
WORKDIR /fixture
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM debian:bookworm-slim
COPY --from=build /fixture/target/release/orange-network-web /usr/local/bin/orange-network-web
USER 65534:65534
CMD ["orange-network-web"]
