FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends software-properties-common && \
    add-apt-repository --yes universe && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      clang \
      curl \
      git \
      libcap-dev \
      libssl-dev \
      pkg-config \
      python3 \
      xz-utils && \
    rm -rf /var/lib/apt/lists/*

USER ubuntu

RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal && \
    /home/ubuntu/.cargo/bin/rustup toolchain install 1.98.1 \
      --profile minimal \
      --component clippy \
      --component rustfmt \
      --component rust-src && \
    /home/ubuntu/.cargo/bin/rustup default 1.98.1

ENV PATH="/home/ubuntu/.cargo/bin:${PATH}"
ENV RUST_BACKTRACE=1

WORKDIR /workspaces
