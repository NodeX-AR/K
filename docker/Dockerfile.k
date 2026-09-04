# Example Dockerfile for running K scripts in a container.
#
# NOT MEASURED: no image size numbers are claimed anywhere for this —
# building and measuring it needs a real Docker daemon, which the
# environment this file was written in doesn't have. Build it yourself
# (`docker build -f docker/Dockerfile.k -t k-lang .` from the repo root)
# and run `docker images k-lang` to get real numbers before quoting any.
#
# Uses --no-default-features to drop the GUI (eframe/rfd/image) from the
# build — a container running scripts has no display to show a GUI on
# anyway, so there's no reason to pay for those dependencies here.

FROM rust:1-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --no-default-features

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/k /usr/local/bin/k
ENTRYPOINT ["k"]
CMD ["--help"]
