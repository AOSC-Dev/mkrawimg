mkrawimg
========

Generate ready-to-flash raw images with AOSC OS for various devices.

Requirements
------------

The following dependencies are required to build and run this tool:

### Library Dependencies (Linked Libraries)

- `libblkid`: for gathering information for block devices, primarily their unique identifiers.
- `liblzma`: for compressing the image file with LZMA2 (xz).
- `libzstd`: for compressing the image file with ZStandard.

### Runtime Dependencies (External commands)

The following executables must be available in the system at runtime:

- `rsync`: For copying the system distribution.
- `mkfs.ext4`, `mkfs.xfs`, `mkfs.btrfs`, `mkfs.vfat`: For making filesystems on partitions.
- `chroot`: For entering the chroot environment of the target container to perform post-installation steps.
- `useradd` from shadow: For adding user to the target container.
- `chpasswd` from shadow: For changing user passwords.
- `partprobe`: For updating the in-kernel partition table cache.

### `binfmt_misc` support and respective binary interpreters

If you intend to build images for devices with a different architecture than your host machine, you must check if your host system supports `binfmt_misc`:

```shell
$ cat /proc/sys/fs/binfmt_misc/status
enabled
```

> [!NOTE]
> Enabling `binfmt_misc` support is beyond the scope of this documentation.

With `binfmt_misc` support enabled, you will have to install `qemu-user-static` (or equivalent packages for your distribution) to allow your system to execute binary executables for the target device's architecture.

Building
--------

Simply run:

```shell
cargo build --release
```
Usage
-----

### List Available Devices

```shell
./target/release/mkrawimg list --format FORMAT
```

While `FORMAT` can be one of the following:

- `pretty`: table format which contains basic information.
- `simple`: simple column-based format splitted by tab character (`'\t'`).

### Build images for one specific device

> [!WARNING]
> Building images requires the root privileges.

```shell
sudo ./target/release/mkrawimg build --variants VARIANTS DEVICE
```

- `VARIANTS`: distribution variants, can be one or more of the `base`, `desktop`, `server`.
  If not specified, all variants will be built.
- `DEVICE`: A string identifying the target device, can be one of the following:
  - Device ID (defined in `device.toml`).
  - Device alias (defined in `device.toml`).
  - The path to the `device.toml` file.

For example:

```shell
sudo ./target/releases/mkrawimg build -V desktop rpi-5b
```

### Build Images for All Devices (in the registry)

```shell
sudo ./target/release/mkrawimg build-all --variants VARIANTS
```

For the advanced usage, please refer to [`Cmdline`](https://cyano.uk/rust-docs/mkrawimg/cli/struct.Cmdline.html).

Adding a new device
-------------------

To add support for a new device, please refer to [`DeviceSpec`](https://cyano.uk/rust-docs/mkrawimg/device/struct.DeviceSpec.html).

Contributing
------------

### Device addition

While CI performs automated checks on submitted device specification files, these checks are not exhaustive. Therefore, we require you to build an image using your specification file to ensure its validity.

License
-------

This repository is licensed under the GNU GPL v3 license.
