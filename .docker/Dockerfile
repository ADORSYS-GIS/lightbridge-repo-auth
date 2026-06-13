# syntax=docker/dockerfile:1
# Multi-stage build → distroless. ring/mimalloc need a libc, so the cc-debian12
# runtime image (not static) is the right base.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --profile prod --bin lightbridge-repo-auth && \
    cp /src/target/prod/lightbridge-repo-auth /usr/local/bin/lightbridge-repo-auth

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /usr/local/bin/lightbridge-repo-auth /usr/local/bin/lightbridge-repo-auth
EXPOSE 3000
USER nonroot
ENTRYPOINT ["/usr/local/bin/lightbridge-repo-auth"]
