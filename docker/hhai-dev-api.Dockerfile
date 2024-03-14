FROM rust:1 as build-step

WORKDIR /src
COPY . .

# Enable debug in release build, thus also enable backtrace
ENV RUSTFLAGS=-g

RUN cargo build --release -p api

FROM gcr.io/distroless/cc

COPY --from=build-step /src/target/release/api /bin/api

CMD ["/bin/api"]
