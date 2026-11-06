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
    systemctl enable systemd-networkd.service && \
    systemctl mask systemd-networkd-wait-online.service \
    systemd-resolved.service \
    e2scrub_reap.service \
    systemd-firstboot.service

# The guest has two virtio consoles. hvc0 is the kernel console: kernel, systemd and the
# supervisor's logs, shown read-only in the app's Logs tab. hvc1 is the session, shown
# in the app's Terminal tab. No login runs on either, or anywhere: the supervisor alone
# starts what a boot runs on hvc1, once the app sends Start over the command plane
# (vsock): the agent's command as ubuntu, a root shell in maintenance mode, or the
# provisioning script in provision mode. The binary is built from guest_supervisor and
# handed in as the build context "supervisor" (see vms/rootfs/Makefile).
RUN systemctl mask serial-getty@hvc0.service serial-getty@hvc1.service getty@tty1.service
COPY --from=supervisor --chmod=755 terrarium-supervisor /usr/local/sbin/terrarium-supervisor
COPY terrarium-supervisor.service /etc/systemd/system/terrarium-supervisor.service
RUN systemctl enable terrarium-supervisor.service

# The command plane's vsock transport, loaded early so the supervisor can listen right away.
RUN printf 'vmw_vsock_virtio_transport\n' > /etc/modules-load.d/vsock.conf

# The shared folders arrive through one virtio-fs device; the supervisor mounts it and
# binds each folder to its path in agent and maintenance boots.
RUN printf 'virtiofs\n' > /etc/modules-load.d/virtiofs.conf

# Virtualization.framework gives Linux guests no usable hardware clock. The clock is set
# early at boot from the Mac's time on the kernel command line; timesyncd keeps it right
# afterwards, e.g. when the VM was paused while the Mac slept.
COPY --chmod=755 clock-from-host /usr/local/sbin/clock-from-host
COPY clock-from-host.service /etc/systemd/system/clock-from-host.service
RUN systemctl enable clock-from-host.service

# The app's Stop goes to the supervisor over the command plane. Before the supervisor
# answers, it presses Virtualization.framework's power button (a PL061 GPIO key) instead;
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
