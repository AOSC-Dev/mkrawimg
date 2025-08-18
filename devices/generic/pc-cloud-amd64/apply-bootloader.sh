echo "Kernel cmdline:"
echo $KERNEL_CMDLINE

echo "Disabling os-prober ..."
echo -e "\nGRUB_DISABLE_OS_PROBER=true" >> /etc/default/grub

echo "Installing grub ..."
grub-install --removable --efi-directory=/efi --target=x86_64-efi

echo "Updating initrd ..."
update-initramfs

echo "Done!"
