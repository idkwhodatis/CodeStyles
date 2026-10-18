# Pass the architecture-specific digest in CI. Never build releases against latest.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}
RUN dnf -y install dnf-plugins-core && \
    dnf config-manager --set-enabled powertools && \
    dnf -y install clang clang-devel llvm-devel cmake \
      alsa-lib-devel fontconfig-devel wayland-devel libX11-devel libX11-xcb \
      libxkbcommon-devel libxkbcommon-x11-devel libxcb-devel vulkan-loader-devel \
      openssl-devel libzstd-devel mesa-libEGL mesa-libGL && \
    rpm -qa | sort > /opt/codestyles-build-packages.txt && dnf clean all
ENV PATH="/root/.cargo/bin:/opt/python/cp312-cp312/bin:${PATH}" \
    RUSTUP_TOOLCHAIN=1.99.1 \
    CARGO_INCREMENTAL=0
# Official Rust distribution, exact repository toolchain. No target-cpu=native.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh && \
    sh /tmp/rustup.sh -y --profile minimal --default-toolchain 1.99.1 && \
    rustup component add rustfmt clippy && rm /tmp/rustup.sh
WORKDIR /src
