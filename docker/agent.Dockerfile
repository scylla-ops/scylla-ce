# The image of the agent. At its first start it joins its App with SCYLLA_URL and
# SCYLLA_JOIN_CODE, and keeps its file in the volume /var/lib/scylla-agent; then it runs the jobs.
# The build context has the Linux binaries in dist/linux-amd64 and dist/linux-arm64.
FROM debian:trixie-slim
ARG TARGETARCH
RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /var/lib/scylla-agent --shell /usr/sbin/nologin scylla-agent
COPY --chmod=0755 dist/linux-${TARGETARCH}/scylla-agent /usr/local/bin/scylla-agent
USER scylla-agent
ENV SCYLLA_AGENT_CONFIG=/var/lib/scylla-agent/agent.json \
    SCYLLA_AGENT_WORKSPACE_ROOT=/var/lib/scylla-agent/work
VOLUME ["/var/lib/scylla-agent"]
ENTRYPOINT ["scylla-agent"]
CMD ["run"]
