// SPDX-License-Identifier: MPL-2.0

mod bin;
mod grub;

use std::{
    path::{Path, PathBuf},
    process,
    str::FromStr,
};

use bin::strip_elf_for_qemu;

use super::utils::{cargo, COMMON_CARGO_ARGS, DEFAULT_TARGET_RELPATH};
use crate::{
    base_crate::new_base_crate,
    bin::{AsterBin, AsterBinType, AsterElfMeta},
    bundle::{Bundle, BundleManifest},
    cli::CargoArgs,
    config_manager::{qemu::QemuMachine, BuildConfig},
    error::Errno,
    error_msg,
    utils::{get_current_crate_info, get_target_directory},
};

pub fn execute_build_command(config: &BuildConfig) {
    let osdk_target_directory = get_target_directory().join(DEFAULT_TARGET_RELPATH);
    if !osdk_target_directory.exists() {
        std::fs::create_dir_all(&osdk_target_directory).unwrap();
    }
    let target_info = get_current_crate_info();
    let bundle_path = osdk_target_directory.join(&target_info.name);

    let _bundle = create_base_and_build(&bundle_path, &osdk_target_directory, &config);
}

pub fn create_base_and_build(
    bundle_path: impl AsRef<Path>,
    osdk_target_directory: impl AsRef<Path>,
    config: &BuildConfig,
) -> Bundle {
    let base_crate_path = osdk_target_directory.as_ref().join("base");
    new_base_crate(
        &base_crate_path,
        &get_current_crate_info().name,
        &get_current_crate_info().path,
    );
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&base_crate_path).unwrap();
    let bundle = do_build(&bundle_path, &osdk_target_directory, &config);
    std::env::set_current_dir(&original_dir).unwrap();
    bundle
}

pub fn do_build(
    bundle_path: impl AsRef<Path>,
    osdk_target_directory: impl AsRef<Path>,
    config: &BuildConfig,
) -> Bundle {
    if let Some(ref initramfs) = config.manifest.initramfs {
        if !initramfs.exists() {
            error_msg!("initramfs file not found: {}", initramfs.display());
            process::exit(Errno::BuildCrate as _);
        }
    };
    let mut bundle = Bundle::new(
        BundleManifest {
            kcmd_args: config.manifest.kcmd_args.clone(),
            initramfs: config.manifest.initramfs.clone(),
            aster_bin: None,
            vm_image: None,
            boot: config.manifest.boot.clone(),
            qemu: config.manifest.qemu.clone(),
            cargo_args: config.cargo_args.clone(),
        },
        &bundle_path,
    );
    info!("Building kernel ELF");
    let aster_elf = build_kernel_elf(&config.cargo_args);

    if matches!(config.manifest.qemu.machine, QemuMachine::Microvm) {
        let stripped_elf = strip_elf_for_qemu(&osdk_target_directory, &aster_elf);
        bundle.add_aster_bin(&stripped_elf);
    }

    // TODO: A boot device is required if we use GRUB. Actually you can boot
    // a multiboot kernel with Q35 machine directly without a bootloader.
    // We are currently ignoring this case.
    if matches!(config.manifest.qemu.machine, QemuMachine::Q35) {
        info!("Building boot device image");
        let bootdev_image = grub::create_bootdev_image(
            &osdk_target_directory,
            &aster_elf,
            config.manifest.initramfs.as_ref(),
            &config,
        );
        bundle.add_vm_image(&bootdev_image);
    }

    bundle
}

fn build_kernel_elf(args: &CargoArgs) -> AsterBin {
    let target_directory = get_target_directory();
    let target_json_path = PathBuf::from_str("x86_64-custom.json").unwrap();

    let mut command = cargo();
    command.arg("build").arg("--target").arg(&target_json_path);
    command.args(COMMON_CARGO_ARGS);
    command.arg("--profile=".to_string() + &args.profile);
    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Cargo build failed");
        process::exit(Errno::ExecuteCommand as _);
    }

    let aster_bin_path = PathBuf::from(target_directory)
        .join(target_json_path.file_stem().unwrap().to_str().unwrap());
    let aster_bin_path = if args.profile == "dev" {
        aster_bin_path.join("debug")
    } else {
        aster_bin_path.join(&args.profile)
    }
    .join(get_current_crate_info().name);

    AsterBin {
        path: aster_bin_path,
        typ: AsterBinType::Elf(AsterElfMeta {
            has_linux_header: false,
            has_pvh_header: false,
            has_multiboot_header: true,
            has_multiboot2_header: true,
        }),
        version: get_current_crate_info().version,
        sha256sum: "TODO".to_string(),
        stripped: false,
    }
}
