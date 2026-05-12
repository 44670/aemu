use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("probe-so") => {
            let Some(path) = args.next() else {
                eprintln!("usage: aemu probe-so <lib.so>");
                std::process::exit(2);
            };
            match aemu::elf_probe::probe_arm_elf(Path::new(&path)) {
                Ok(probe) => print_probe(&probe),
                Err(err) => {
                    eprintln!("probe failed: {err}");
                    std::process::exit(1);
                }
            }
        }
        Some("probe-apk") => {
            let Some(path) = args.next() else {
                eprintln!("usage: aemu probe-apk <app.apk>");
                std::process::exit(2);
            };
            if let Err(err) = probe_apk(Path::new(&path)) {
                eprintln!("probe failed: {err}");
                std::process::exit(1);
            }
        }
        Some("plan-apk") => {
            let Some(path) = args.next() else {
                eprintln!("usage: aemu plan-apk <app.apk>");
                std::process::exit(2);
            };
            match aemu::apk_plan::analyze_apk(Path::new(&path)) {
                Ok(plan) => print_apk_plan(&plan),
                Err(err) => {
                    eprintln!("plan failed: {err}");
                    std::process::exit(1);
                }
            }
        }
        Some("sdl2-shell") => run_sdl2_shell(args.collect()),
        _ => {
            eprintln!("usage:");
            eprintln!("  aemu probe-so <lib.so>");
            eprintln!("  aemu probe-apk <app.apk>");
            eprintln!("  aemu plan-apk <app.apk>");
            eprintln!("  aemu sdl2-shell [--frames N] [--width W] [--height H]");
        }
    }
}

fn print_probe(probe: &aemu::elf_probe::ElfProbe) {
    println!("path: {}", probe.path.display());
    println!("machine: {}", probe.machine);
    println!("eabi: {}", probe.eabi);
    if probe.attributes.is_empty() {
        println!("attributes: <none>");
    } else {
        println!("attributes:");
        for attr in &probe.attributes {
            println!("  {}: {}", attr.name, attr.value);
        }
    }
    println!("requires:");
    println!("  armv7_or_newer: {}", probe.requires_armv7_or_newer());
    println!("  thumb2: {}", probe.requires_thumb2());
    println!("  vfp: {}", probe.requires_vfp());
    println!("  neon: {}", probe.requires_neon());
}

fn probe_apk(path: &Path) -> Result<(), String> {
    let zip_entries = aemu::zip_probe::read_zip_entries(path)
        .map_err(|err| format!("failed to inspect ZIP metadata: {err}"))?;
    let bytes = std::fs::read(path).map_err(|err| format!("failed to read APK: {err}"))?;
    let libs: Vec<_> = zip_entries
        .iter()
        .filter(|entry| entry.name.starts_with("lib/") && entry.name.ends_with(".so"))
        .collect();
    if libs.is_empty() {
        println!("no native libraries found in {}", path.display());
        return Ok(());
    }

    println!("apk: {}", path.display());
    println!("native libraries: {}", libs.len());
    for entry in libs {
        println!();
        println!("entry: {}", entry.name);
        let saved = entry
            .saved_percent()
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "n/a".to_string());
        println!(
            "zip: method {}, compressed {} bytes, uncompressed {} bytes, saved {}",
            entry.compression, entry.compressed_size, entry.uncompressed_size, saved
        );
        let extracted = aemu::zip_probe::extract_parsed_zip_entry(&bytes, entry)
            .map_err(|err| format!("failed to extract {}: {err}", entry.name))?;
        match aemu::elf_probe::probe_arm_elf_bytes(
            PathBuf::from(format!("{}!{}", path.display(), entry.name)),
            &extracted,
        ) {
            Ok(probe) => print_probe(&probe),
            Err(err) => println!("  probe failed: {err}"),
        }
    }

    Ok(())
}

fn print_apk_plan(plan: &aemu::apk_plan::ApkRunPlan) {
    println!("apk: {}", plan.path.display());
    println!("runtime target: Android 4.x ARMv6 / armeabi");
    println!("native libraries: {}", plan.native_libraries.len());

    let abi_counts = plan.abi_counts();
    if abi_counts.is_empty() {
        println!("abi groups: <none>");
    } else {
        println!("abi groups:");
        for (abi, count) in abi_counts {
            println!("  {abi}: {count}");
        }
    }

    println!(
        "selected ABI: {}",
        if plan.has_target_abi() {
            aemu::apk_plan::ARMV6_TARGET_ABI
        } else {
            "<none>"
        }
    );
    println!(
        "result: {}",
        if plan.is_armv6_runnable() {
            "ready for ARMv6 ELF loader"
        } else {
            "blocked for ARMv6 interpreter"
        }
    );

    let blockers = plan.summary_blockers();
    if !blockers.is_empty() {
        println!("blockers:");
        for blocker in blockers {
            println!("  - {blocker}");
        }
    }

    if !plan.native_libraries.is_empty() {
        println!("libraries:");
        for library in &plan.native_libraries {
            let status = if library.armv6_blockers.is_empty() {
                "compatible"
            } else {
                "blocked"
            };
            println!("  {} [{}]: {status}", library.entry_name, library.abi);
        }
    }
}

#[cfg(feature = "sdl2")]
fn run_sdl2_shell(args: Vec<String>) {
    match parse_sdl2_shell_args(args) {
        Ok((config, max_frames)) => {
            if let Err(err) = aemu::sdl_shell::run_debug_shell(config, max_frames) {
                eprintln!("sdl2 shell failed: {err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!("usage: aemu sdl2-shell [--frames N] [--width W] [--height H]");
            std::process::exit(2);
        }
    }
}

#[cfg(feature = "sdl2")]
fn parse_sdl2_shell_args(
    args: Vec<String>,
) -> Result<(aemu::host::HostConfig, Option<u64>), String> {
    let mut config = aemu::host::HostConfig::default();
    let mut max_frames = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--frames" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--frames needs a numeric value".to_string())?;
                max_frames = Some(
                    value
                        .parse()
                        .map_err(|_| format!("invalid --frames value: {value}"))?,
                );
            }
            "--width" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--width needs a numeric value".to_string())?;
                config.width = value
                    .parse()
                    .map_err(|_| format!("invalid --width value: {value}"))?;
            }
            "--height" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--height needs a numeric value".to_string())?;
                config.height = value
                    .parse()
                    .map_err(|_| format!("invalid --height value: {value}"))?;
            }
            _ => return Err(format!("unknown sdl2-shell argument: {arg}")),
        }
    }

    Ok((config, max_frames))
}

#[cfg(not(feature = "sdl2"))]
fn run_sdl2_shell(_args: Vec<String>) {
    eprintln!("sdl2-shell requires rebuilding with --features sdl2");
    std::process::exit(2);
}
