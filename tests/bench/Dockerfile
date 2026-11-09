# Benchmark environment: Debian's xsltproc and oxslt, both native code on the
# same machine. Build for the host architecture (arm64 on Apple Silicon, in
# Colima's VM) so nothing runs under emulation.
#
#   docker build -t oxslt-bench tests/bench
FROM rust:1-bookworm

RUN apt-get update \
 && apt-get install -y --no-install-recommends xsltproc hyperfine time \
 && rm -rf /var/lib/apt/lists/*
