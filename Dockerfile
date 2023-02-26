FROM docker.io/library/rust:1.67.1-alpine3.17 as builder

RUN apk add --no-cache musl-dev

WORKDIR /wd

COPY . /wd

RUN cargo build --bins --release

FROM scratch

COPY --from=builder /wd/target/release/bloggy /
COPY --from=builder /wd/cert /cert
COPY --from=builder /wd/public /public
COPY --from=builder /wd/posts /posts
COPY --from=builder /wd/themes /themes

CMD ["./bloggy"]

