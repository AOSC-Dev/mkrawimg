echo "Kernel cmdline:"
echo $KERNEL_CMDLINE

echo "Disabling os-prober ..."
echo -e "\nGRUB_DISABLE_OS_PROBER=true" >> /etc/default/grub

echo "Installing grub ..."
grub-install --removable --efi-directory=/efi --target=x86_64-efi

echo "Updating initrd ..."
update-initramfs

echo "Finding kernel ..."
KERNELS=()
for version in `find /usr/lib/modules -mindepth 1 -maxdepth 1 -type d -printf '%P\n' | sort -V` ; do
        if [ -f "/usr/lib/modules/${version}/modules.dep" ] && [ -f "/usr/lib/modules/${version}/modules.order" ] && [ -f "/usr/lib/modules/${version}/modules.builtin" ]; then
                KERNELS+=("$version")
        fi
done

echo "Generating grub.cfg ..."
cat > /boot/grub/grub.cfg << EOF
insmod ext2
insmod xfs
insmod btrfs
search --no-floppy --fs-uuid --set=root ${ROOT_FSUUID}
set timeout=0
set lang=en_US

menuentry 'AOSC OS' {
	linux	/boot/vmlinuz-${KERNELS[0]} $KERNEL_CMDLINE quiet splash
	initrd	/boot/devena-firstboot.img
}
EOF

echo "Done!"
