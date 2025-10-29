#!/bin/bash

gen_cmdline() {
	if [ "x$ROOT_PARTUUID" = x ] ; then
		echo "ROOT_PARTUUID is empty."
		exit 1
	fi
	echo "console=serial0,115200 console=tty1 root=PARTUUID=$ROOT_PARTUUID rw rootwait fsck.repair=yes quiet splash" | tee /boot/rpi/cmdline.txt
}

kernels=($(ls -A /usr/lib/rpi64/kernel/))
echo "Installing kernel ..."
cp -av /usr/lib/rpi64/kernel/"${kernels[-1]}"/* /boot/rpi/

echo "Installing boot firmware ..."
cp -av /usr/lib/rpi64/boot/* /boot/rpi/

echo "Installing boot configuration ..."
cp -av /usr/lib/rpi64/config/* /boot/rpi/

echo "Generating cmdline.txt ..."
gen_cmdline


echo "Creating devena-cfg.txt ..."
cat > /boot/rpi/devena-cfg.txt << EOF
# Initramfs configuration for devena-firstboot.
# DO NOT EDIT! This file will be removed after finishing the first boot
# setup.
[pi3]
initramfs devena-initrd-rpi4.img followkernel
[pi4]
initramfs devena-initrd-rpi4.img followkernel
[pi5]
initramfs devena-initrd-rpi5.img followkernel
EOF

if ! grep -q "^include devena-cfg.txt" /boot/rpi/distcfg.txt ; then
	echo "-- Including devena-cfrg.txt in distcfg.txt ..."
	echo "include devena-cfg.txt" >> /boot/rpi/distcfg.txt
fi

echo "Done!"
