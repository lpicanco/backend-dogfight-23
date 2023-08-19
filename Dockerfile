FROM rust:1.71.1 as builder
WORKDIR /app
RUN USER=root cargo new backend-dogfight-23
WORKDIR /app/backend-dogfight-23
COPY Cargo.toml Cargo.lock ./
RUN cargo build --release
RUN rm -rf src

COPY src src

RUN touch src/main.rs
RUN cargo build --release


FROM debian:bullseye-slim
COPY --from=builder /app/backend-dogfight-23/target/release/backend-dogfight-23 /usr/local/bin/

CMD ["backend-dogfight-23"]
