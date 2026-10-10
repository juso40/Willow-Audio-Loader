FROM python:3.14-slim-bookworm

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/opt/llvm-mingw/bin:$PATH

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gcc git libc6-dev xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --profile minimal --default-toolchain 1.96.0 \
    && rustup target add i686-pc-windows-gnullvm

RUN curl -fsSL https://github.com/mstorsjo/llvm-mingw/releases/download/20260908/llvm-mingw-20260908-ucrt-ubuntu-22.04-x86_64.tar.xz \
        -o /tmp/llvm-mingw.tar.xz \
    && mkdir -p /opt/llvm-mingw \
    && tar -xJf /tmp/llvm-mingw.tar.xz -C /opt/llvm-mingw --strip-components=1 \
    && rm /tmp/llvm-mingw.tar.xz

WORKDIR /work
CMD ["python", "scripts/build_mod.py"]
