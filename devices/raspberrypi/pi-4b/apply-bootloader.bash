#!/bin/bash

gen_cmdline() {
	if [ "x$ROOT_PARTUUID" = x ] ; then
		echo "ROOT_PARTUUID is empty."
		exit 1
	fi
	echo "console=serial0,115200 console=tty1 root=PARTUUID=$ROOT_PARTUUID rw rootwait fsck.repair=yes" | tee /boot/rpi/cmdline.txt
}

kernel=$(ls /usr/lib/rpi64/kernel/)
echo "Installing kernel ..."
cp -av /usr/lib/rpi64/kernel/"$kernel"/* /boot/rpi/

echo "Installing boot firmware ..."
cp -av /usr/lib/rpi64/boot/* /boot/rpi/

echo "Generating cmdline.txt ..."
gen_cmdline

echo "Done!"
