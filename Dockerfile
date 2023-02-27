# syntax=docker/dockerfile:1
FROM docker.io/library/rust:1.67.1-alpine3.17 as builder
RUN apk add --no-cache musl-dev
WORKDIR /wd
COPY . /wd
ENV TARGET x86_64-unknown-linux-musl
RUN rustup target add "$TARGET"
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/wd/target \
    cargo build \
      --bins \
      --locked \
      --release \
      --target "$TARGET"

FROM gcr.io/distroless/cc-debian11
COPY --from=builder /wd/target/x86_64-unknown-linux-musl/release/bloggy /
COPY --from=builder /wd/cert /cert
COPY --from=builder /wd/public /public
COPY --from=builder /wd/posts /posts
COPY --from=builder /wd/themes /themes
EXPOSE 3443
CMD ["./bloggy"]
