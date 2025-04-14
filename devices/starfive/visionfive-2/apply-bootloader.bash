#!/bin/bash

echo "Generating vf2_uEnv.txt ..."
cat > /efi/vf2_uEnv.txt <<- EOF
	fdt_high=0xffffffffffffffff
	initrd_high=0xffffffffffffffff
	kernel_addr_r=0x40200000
	kernel_comp_addr_r=0x5a000000
	kernel_comp_size=0x4000000
	fdt_addr_r=0x46000000
	ramdisk_addr_r=0x46100000
	boot2=if test \${chip_vision} = B; then setenv fdtfile starfive/jh7110-starfive-visionfive-2-v1.3b.dtb; else setenv fdtfile starfive/jh7110-starfive-visionfive-2-v1.2a.dtb; fi; sysboot \${bootdev} \${devnum}:4 any \${scriptaddr} /extlinux/extlinux.conf
EOF

echo "Configuring U-Boot ..."
cat > /etc/default/u-boot <<- EOF
	U_BOOT_UPDATE="true"
	U_BOOT_PARAMETERS="rw console=tty0 console=ttyS0,115200 earlycon rootwait stmmaceth=chain_mode:1 selinux=0"
	U_BOOT_ROOT="root=PARTUUID=${ROOT_PARTUUID}"
	U_BOOT_FDT_DIR="/dtbs/dtbs-"
	U_BOOT_SYNC_DTBS=true
EOF

echo "Syncing dtbs to /boot ..."
_KERNEL=$(ls boot/vmlinux-* | sed 's#boot/vmlinux-##')
/etc/kernel/postinst.d/zz-u-boot-menu ${_KERNEL}
