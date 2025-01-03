#!/bin/bash

case "$DEVICE_ID" in
	visionfive-2)
	DTB_NAME=jh7110-starfive-visionfive-2-v1.3b
	;;
	*)
	DTB_NAME=jh7110-starfive-visionfive-2-v1.2a
	;;
esac

echo "Generating uEnv.txt ..."
cat > /boot/u-boot/uEnv.txt <<- EOF
	fdt_high=0xffffffffffffffff
	initrd_high=0xffffffffffffffff
	kernel_addr_r=0x40200000
	kernel_comp_addr_r=0x5a000000
	kernel_comp_size=0x4000000
	fdt_addr_r=0x46000000
	ramdisk_addr_r=0x46100000
	#Move distro to first boot to speed up booting
	boot_targets=distro mmc0 dhcp
	#Fix wrong fdtfile name
	fdtfile=starfive/${DTB_NAME}.dtb
	#Fix missing bootcmd
	bootcmd=run load_distro_uenv;run bootcmd_distro
EOF

echo "Configuring U-Boot ..."
cat > /etc/default/u-boot <<- EOF
	U_BOOT_UPDATE="true"
	U_BOOT_PARAMETERS="rw console=tty0 console=ttyS0,115200 earlycon rootwait stmmaceth=chain_mode:1 selinux=0"
	U_BOOT_ROOT="root=PARTUUID=${ROOT_PARTUUID}"
	U_BOOT_FDT_DIR="/dtbs/dtbs-"
	U_BOOT_SYNC_DTBS=true
EOF

echo "Syncing dtbs to /boot/u-boot ..."
_KERNEL=$(ls boot/vmlinux-* | sed 's#boot/vmlinux-##')
/etc/kernel/postinst.d/zz-u-boot-menu ${_KERNEL}
