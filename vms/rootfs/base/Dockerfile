FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

# Install necessary packages
RUN apt-get update && \
    apt-get -o Dpkg::Options::=--force-confold -o Dpkg::Options::=--force-confdef install -y \
    systemd systemd-sysv dbus udev sudo ubuntu-standard \
    coreutils curl wget ca-certificates git \
    build-essential pkg-config python3 python3-pip tcpdump g++ cmake make libpcap-dev \
    jq unzip nano

# Set up systemd
RUN systemctl set-default multi-user.target && \
    systemctl enable serial-getty@hvc0.service && \
    systemctl enable systemd-networkd.service && \
    systemctl mask systemd-networkd-wait-online.service \
    systemd-resolved.service \
    e2scrub_reap.service \
    systemd-firstboot.service

# The app's terminal is the virtio console (hvc0), logged in there without a password:
# ubuntu in agent mode, root in maintenance mode (kernel flag agent_toolbox.mode=maintenance).
COPY --chmod=755 console-autologin /usr/local/sbin/console-autologin
RUN mkdir -p /etc/systemd/system/serial-getty@hvc0.service.d && \
    printf '[Service]\nExecStart=\nExecStart=-/usr/local/sbin/console-autologin $TERM\n' \
    > /etc/systemd/system/serial-getty@hvc0.service.d/autologin.conf

# Keep orchestrator's network configuration
# RUN printf '[Match]\nName=eth0\n\n[Network]\nDHCP=no\nKeepConfiguration=yes\n' \
#     > /etc/systemd/network/10-eth0.network

# Install runtimes
RUN ln -sf /usr/bin/python3 /usr/bin/python
RUN curl -fsSL https://releases.astral.sh/github/uv/releases/download/0.10.11/uv-aarch64-unknown-linux-gnu.tar.gz \
    | tar -xz -C /usr/bin --strip-components=1 \
    uv-aarch64-unknown-linux-gnu/uv uv-aarch64-unknown-linux-gnu/uvx
# No node here. The nodesource package declares Conflicts: npm, so a subject
# environment that asks apt for nodejs and npm cannot be provisioned on top of
# this image. Kinds that need node install it themselves.

# Add ubuntu user
RUN id ubuntu >/dev/null 2>&1 || useradd -m -s /bin/bash ubuntu;
# RUN usermod -aG sudo ubuntu
# RUN echo "ubuntu ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/ubuntu \
#     && chmod 0440 /etc/sudoers.d/ubuntu
# USER ubuntu
# WORKDIR /home/ubuntu
# RUN mkdir -p .config .local/bin .local/share .cache .npm .ssh Documents Downloads Pictures Music Videos Desktop Public Templates && \
#     chmod 700 .ssh && \
#     touch .bash_history .zsh_history && \
#     printf 'prefix=/home/ubuntu/.local\n' > .npmrc && \
#     printf 'if [ -d "$HOME/.local/bin" ]; then PATH="$HOME/.local/bin:$PATH"; fi\n' | tee .profile .bashrc >/dev/null && \
#     git config --global user.name  "John Smith" && \
#     git config --global user.email "john.smith@mymail.com" && \
#     git config --global credential.helper store && \
#     printf 'https://johns:FooBar1234@github.com\nhttps://johnsm1988:FooBar1234@gitlab.com\n' > .git-credentials && \
#     ssh-keygen -t ed25519 -C john.smith@mymail.com -f .ssh/id_ed25519 -N "" && \
#     ssh-keygen -t rsa -b 4096 -C john.smith@mymail.com -f .ssh/id_rsa -N "" && \
#     gpg --batch --passphrase "" --quick-generate-key john.smith@mymail.com
# USER root

# Fix DNS
# RUN printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' > /etc/resolv.conf.static && \
#     printf 'L+ /etc/resolv.conf - - - - /etc/resolv.conf.static\n' > /etc/tmpfiles.d/dns.conf && \
#     printf 'd /results/executor 0755 root root - -\n' > /etc/tmpfiles.d/executor_logs.conf

# Copy custom files and services
# COPY files/executor /usr/local/bin/executor
# COPY files/executor.service /etc/systemd/system/executor.service
# COPY files/payload.mount /etc/systemd/system/payload.mount
# COPY files/results.mount /etc/systemd/system/results.mount
# RUN systemctl enable executor.service payload.mount results.mount

# Set CMD for debug
CMD ["/sbin/init"]
