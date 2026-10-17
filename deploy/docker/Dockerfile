# The Jarvis License Server in a container (build from the root of the repository:  docker build -f deploy/docker/Dockerfile -t jarvis-license .)
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p jarvis-license-server && strip target/release/jarvis-license-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /data jarvis-license && install -d -m 700 -o jarvis-license /data
COPY --from=build /src/target/release/jarvis-license-server /usr/local/bin/jarvis-license-server
COPY deploy/docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 755 /usr/local/bin/entrypoint.sh
USER jarvis-license
# everything that must survive lives in /data: the database and the two secret files (back it up, keep the key files OFFLINE too)
VOLUME /data
ENV JARVIS_LS_DB=/data/licenses.sqlite \
    JARVIS_LS_SIGNING_KEY_FILE=/data/signing.key \
    JARVIS_LS_PEPPER_FILE=/data/pepper.key \
    JARVIS_LS_BIND=0.0.0.0:8787
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --retries=3 CMD curl -fsS http://127.0.0.1:8787/health || exit 1
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["serve"]
