# Ghidra + ghidra-cli, for headless/agent-driven reverse engineering.
FROM eclipse-temurin:21-jdk

RUN apt-get update && apt-get install -y --no-install-recommends \
        curl unzip ca-certificates git build-essential pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

ARG GHIDRA_URL=https://github.com/NationalSecurityAgency/ghidra/releases/download/Ghidra_12.1.3_build/ghidra_12.1.3_PUBLIC_20260817.zip
RUN curl -fsSL "$GHIDRA_URL" -o /tmp/ghidra.zip \
 && unzip -q /tmp/ghidra.zip -d /opt \
 && rm /tmp/ghidra.zip \
 && mv /opt/ghidra_* /opt/ghidra
ENV GHIDRA_INSTALL_DIR=/opt/ghidra
ENV PATH=/opt/ghidra/support:$PATH

RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH=/root/.cargo/bin:$PATH

RUN git clone --depth 1 https://github.com/akiselev/ghidra-cli /src/ghidra-cli \
 && cd /src/ghidra-cli \
 && cargo install --path . \
 && rm -rf /src /root/.cargo/registry

WORKDIR /work
CMD ["/bin/bash"]
