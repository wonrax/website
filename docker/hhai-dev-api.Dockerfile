FROM rust:1 as build-step

WORKDIR /src
COPY . .

# Enable debug in release build, thus also enable backtrace
ENV RUSTFLAGS=-g

RUN cargo build --release -p api

FROM debian:bookworm-slim

COPY --from=build-step /src/target/release/api /bin/api
RUN apt-get -y update \
    && apt-get install -y libssl3 ca-certificates libpq-dev

CMD ["/bin/api"]
