# shellcheck shell=bash
set -euo pipefail

echo "Disabling os-prober ..."
echo -e "\nGRUB_DISABLE_OS_PROBER=true" >>/etc/default/grub

echo "Disabling GRUB boot menu ..."
echo -e "\nGRUB_TIMEOUT=0" >>/etc/default/grub

echo "Allowing GRUB to use UUIDs ..."
# Without this grub will try to generate things like root=/dev/loop0p3
{
    echo -e "\nGRUB_DISABLE_UUID=false"
    echo -e "\nGRUB_DISABLE_LINUX_UUID=false"
    echo -e "\nGRUB_DISABLE_LINUX_PARTUUID=false"
} >>/etc/default/grub

echo "Injecting kernel parameters ..."
echo -e "\nGRUB_CMDLINE_LINUX='$KERNEL_CMDLINE \${ignition_firstboot}'" >>/etc/default/grub

echo "Injecting ignition.firstboot GRUB configuration ..."
cat >/etc/grub.d/09_podman-vm <<EOG
cat <<EOF
set ignition_firstboot=""
if [ -f "/ignition.firstboot" ]; then
    source "/ignition.firstboot"
    set ignition_firstboot="ignition.firstboot"
fi
EOF
EOG
chmod -v +x /etc/grub.d/09_podman-vm
cat >/boot/ignition.firstboot <<EOF
EOF

echo "Stubbing /dev/disk ..."
# /dev/disk/by-{partuuid,uuid}/UUID must be created or else GRUB will not use UUIDs
# https://github.com/AOSC-Dev/grub/blob/1e1ce61d967cc992b44c5206cc0cfe21cd44c7ff/util/grub.d/10_linux.in#L57
! [[ -e /dev/disk ]] || {
    echo "Error: /dev/disk already exists"
    exit 1
}
rootDev="$(grub-probe --target=device /)"
mkdir -vp /dev/disk/by-uuid/"$(grub-probe --device "$rootDev" --target=fs_uuid)"
mkdir -vp /dev/disk/by-partuuid/"$(grub-probe --device "$rootDev" --target=partuuid)"

echo "Installing GRUB to $LOOPDEV ..."
grub-install --target=i386-pc "$LOOPDEV"

echo "Updating initrd ..."
update-initramfs

echo "Updating GRUB configuration ..."
update-grub

echo "Removing /dev/disk stub ..."
rm -vrf /dev/disk

echo "Removing /etc/localtime ..."
# ignition will try to create this
# this must be postponed to apply-bootloader rather than in postinst
# to avoid something recreating this file
rm -v /etc/localtime

echo "Done!"
