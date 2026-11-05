FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

# Install necessary packages. linux-modules must match the kernel pinned in
# scripts/build-guest.sh; change both together.
RUN apt-get update && \
    apt-get -o Dpkg::Options::=--force-confold -o Dpkg::Options::=--force-confdef install -y \
    systemd systemd-sysv systemd-timesyncd dbus udev sudo ubuntu-standard \
    linux-modules-6.8.0-147-generic kmod \
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
# Provision mode (agent_toolbox.mode=provision) runs no login; the provisioning script's
# output goes to the console instead.
COPY --chmod=755 console-autologin /usr/local/sbin/console-autologin
RUN mkdir -p /etc/systemd/system/serial-getty@hvc0.service.d && \
    printf '[Unit]\nConditionKernelCommandLine=!agent_toolbox.mode=provision\n\n[Service]\nExecStart=\nExecStart=-/usr/local/sbin/console-autologin $TERM\n' \
    > /etc/systemd/system/serial-getty@hvc0.service.d/autologin.conf

# Provision mode runs the instance's provisioning script from a folder the app shares,
# then powers off. The app boots it before the first agent boot; the script installs
# the agent harness.
COPY --chmod=755 agent-toolbox-provision /usr/local/sbin/agent-toolbox-provision
COPY agent-toolbox-provision.service /etc/systemd/system/agent-toolbox-provision.service
RUN systemctl enable agent-toolbox-provision.service

# Virtualization.framework gives Linux guests no usable hardware clock. The clock is set
# early at boot from the Mac's time on the kernel command line; timesyncd keeps it right
# afterwards, e.g. when the VM was paused while the Mac slept.
COPY --chmod=755 clock-from-host /usr/local/sbin/clock-from-host
COPY clock-from-host.service /etc/systemd/system/clock-from-host.service
RUN systemctl enable clock-from-host.service

# The app's Stop presses Virtualization.framework's power button (a PL061 GPIO key);
# gpio_keys reports it and systemd-logind powers off.
RUN printf 'gpio_keys\n' > /etc/modules-load.d/power-button.conf

# Network: the app runs the gateway (and DNS) at 192.168.127.1 on the virtio NIC and
# forwards connections from the host. Static IPv4 only, so the app needs no DHCP server.
# resolv.conf is a tmpfiles link because Docker bind-mounts it during the build.
RUN printf '[Match]\nType=ether\n\n[Network]\nAddress=192.168.127.2/24\nGateway=192.168.127.1\nLinkLocalAddressing=no\nIPv6AcceptRA=no\n' \
    > /etc/systemd/network/10-vm.network && \
    printf 'nameserver 192.168.127.1\n' > /etc/resolv.conf.static && \
    printf 'L+ /etc/resolv.conf - - - - /etc/resolv.conf.static\n' > /etc/tmpfiles.d/dns.conf

# Install runtimes
RUN ln -sf /usr/bin/python3 /usr/bin/python
RUN curl -fsSL https://releases.astral.sh/github/uv/releases/download/0.10.11/uv-aarch64-unknown-linux-gnu.tar.gz \
    | tar -xz -C /usr/bin --strip-components=1 \
    uv-aarch64-unknown-linux-gnu/uv uv-aarch64-unknown-linux-gnu/uvx
# No node: provisioning scripts that need it install their own.

# The unprivileged user that runs the agent (ubuntu:24.04 already ships it).
RUN id ubuntu >/dev/null 2>&1 || useradd -m -s /bin/bash ubuntu

CMD ["/sbin/init"]
