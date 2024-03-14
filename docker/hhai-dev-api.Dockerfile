FROM rust:1 as build-step

WORKDIR /src
COPY . .
RUN cargo build --release -p api

FROM debian:bookworm-slim

COPY --from=build-step /src/target/release/api /bin/api
RUN apt-get update && apt-get install libssl3

CMD ["/bin/api"]
