# shellcheck shell=bash
set -euo pipefail

echo "Enabling autologin ..."
# enable autologin to make debugging easier
mkdir -vp \
    /etc/systemd/system/serial-getty@.service.d \
    /etc/systemd/system/getty@.service.d
cat >/etc/systemd/system/serial-getty@.service.d/10-autologin.conf <<EOF
[Service]
ExecStart=
ExecStart=-/usr/sbin/agetty --autologin root --noclear %I \$TERM
EOF
cat >/etc/systemd/system/getty@.service.d/10-autologin.conf <<EOF
[Service]
ExecStart=
ExecStart=-/usr/sbin/agetty --autologin root --noclear %I \$TERM
EOF

echo "Including /etc/ssh/sshd_config.d as sshd_config ..."
echo -e "\nInclude /etc/ssh/sshd_config.d/*" >>/etc/ssh/sshd_config

echo "Disabling authfail penalty ..."
cat >/etc/ssh/sshd_config.d/99-podman-vm-penalty.conf <<EOF
# According to the upstream, there is some problem on macOS
# with connecting to ssh to early and
# it seems to count that as auth failure locking us out.
PerSourcePenalties authfail:1s
PerSourcePenaltyExemptList 0.0.0.0/0
EOF

echo "Including authorized_keys.d/ignition as authorized keys ..."
cat >/etc/ssh/sshd_config.d/99-podman-vm-auth.conf <<EOF
AuthorizedKeysFile .ssh/authorized_keys .ssh/authorized_keys.d/ignition
EOF

echo "Relaxing inotify instances limit ..."
cat >/etc/sysctl.d/10-podman-vm.conf <<EOF
fs.inotify.max_user_instances=524288
EOF

echo "Enabling ignition ..."
cat >/etc/dracut.conf.d/10-podman-vm.conf <<EOF
add_dracutmodules+=" ignition "
EOF

echo "Adding ignition sequential boot marker ..."
cat >/usr/lib/systemd/system/ignition-seq-boot-marker.service <<EOF
[Unit]
Description=Ignition Sequential Boot Marker

[Service]
ExecStart=/usr/bin/rm -vrf /boot/ignition.firstboot
WorkingDirectory=/
User=root
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
ln -vs ../ignition-seq-boot-marker.service /usr/lib/systemd/system/multi-user.target.wants/

# we use a custom boot ready sender along with the podman injected sender
# because in our case the port is /dev/vport0p1 but they hard-coded vport1p1
# FIXME: upstream this
echo "Adding boot ready sender ..."
cat >/usr/lib/systemd/system/aosc-ready.service <<EOF
[Unit]
Description=Podman Boot Ready Sender (AOSC OS)
After=systemd-user-sessions.service sshd.socket sshd.service

[Service]
ExecStart=/usr/bin/sh -c 'echo Ready | tee /dev/virtio-ports/org.fedoraproject.port.0'
WorkingDirectory=/
User=root
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
ln -vs ../aosc-ready.service /usr/lib/systemd/system/multi-user.target.wants/

echo "Removing aosc user ..."
# ignition may create a user with UID 1000
userdel -rf aosc

echo "Adding sudo group ..."
# ignition will try to add sudo group to the created user
groupadd sudo -U root

echo "Done!"
