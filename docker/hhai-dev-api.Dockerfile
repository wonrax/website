# This stage is divided into 2 substages which uses cargo chef to cache the
# dependency build step
FROM rust:latest AS rust-builder
RUN apt-get -y update \
    && apt-get install -y libssl3 ca-certificates libpq-dev libxml2-dev libclang-dev

RUN curl -L --proto '=https' --tlsv1.2 -sSf \
        https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
      | bash
RUN cargo binstall cargo-chef -y
WORKDIR /src

# Enable debug in release build, thus also enable backtrace
ENV RUSTFLAGS=-g

FROM rust-builder AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM rust-builder AS build-step
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --workspace --release --recipe-path recipe.json
COPY . .
RUN cargo build --release -p api

FROM debian:bookworm-slim

RUN apt-get -y update \
    && apt-get install -y libssl3 ca-certificates libpq-dev

COPY --from=build-step /src/target/release/api /bin/api

CMD ["/bin/api"]
