FROM rust:1.71 as build-step

WORKDIR /src
COPY . .
RUN cargo build --release -p api

FROM gcr.io/distroless/cc

COPY --from=build-step /src/target/release/api /bin/api

CMD ["/bin/api"]
