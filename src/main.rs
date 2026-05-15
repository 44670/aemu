use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use aemu::armv6::{Cpu, Isa, Memory, Trap, VecMemory};

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
        Some("imports-so") => {
            let Some(path) = args.next() else {
                eprintln!("usage: aemu imports-so <lib.so> [--limit N|--all]");
                std::process::exit(2);
            };
            let limit = match parse_import_limit(args.collect()) {
                Ok(limit) => limit,
                Err(err) => {
                    eprintln!("{err}");
                    eprintln!("usage: aemu imports-so <lib.so> [--limit N|--all]");
                    std::process::exit(2);
                }
            };
            if let Err(err) = imports_so(Path::new(&path), limit) {
                eprintln!("imports failed: {err}");
                std::process::exit(1);
            }
        }
        Some("imports-apk") => {
            let Some(path) = args.next() else {
                eprintln!("usage: aemu imports-apk <app.apk> [--limit N|--all]");
                std::process::exit(2);
            };
            let limit = match parse_import_limit(args.collect()) {
                Ok(limit) => limit,
                Err(err) => {
                    eprintln!("{err}");
                    eprintln!("usage: aemu imports-apk <app.apk> [--limit N|--all]");
                    std::process::exit(2);
                }
            };
            if let Err(err) = imports_apk(Path::new(&path), limit) {
                eprintln!("imports failed: {err}");
                std::process::exit(1);
            }
        }
        Some("link-apk") => {
            let (path, config, limit) = match parse_link_apk_args(args.collect()) {
                Ok(parsed) => parsed,
                Err(err) => {
                    eprintln!("{err}");
                    eprintln!("usage: aemu link-apk <app.apk> [--abi ABI] [--limit N|--all]");
                    std::process::exit(2);
                }
            };
            match aemu::native_loader::load_apk_native_libraries_with_config(&path, &config) {
                Ok(report) => print_native_link_report(&report, limit),
                Err(err) => {
                    eprintln!("link failed: {err}");
                    std::process::exit(1);
                }
            }
        }
        Some("run-apk-native") => {
            let (path, config, max_steps, options) = match parse_run_apk_native_args(args.collect())
            {
                Ok(parsed) => parsed,
                Err(err) => {
                    eprintln!("{err}");
                    eprintln!(
                        "usage: aemu run-apk-native <app.apk> [--abi ABI] [--cpu-backend aemu|qemu-armv7a-tcg] [--steps N] [--launch] [--until-swap] [--gles-summary] [--sdl2] [--sdl2-live] [--sdl2-frames N] [--ws ADDR]"
                    );
                    std::process::exit(2);
                }
            };
            if let Err(err) = run_apk_native(&path, &config, max_steps, options) {
                eprintln!("native run failed: {err}");
                std::process::exit(1);
            }
        }
        Some("qemu-tcg-smoke") => {
            if let Err(err) = run_qemu_tcg_smoke() {
                eprintln!("qemu tcg smoke failed: {err}");
                std::process::exit(1);
            }
        }
        Some("cpu-compare-smoke") => {
            if let Err(err) = run_cpu_compare_smoke() {
                eprintln!("cpu compare smoke failed: {err}");
                std::process::exit(1);
            }
        }
        Some("sdl2-shell") => run_sdl2_shell(args.collect()),
        _ => {
            eprintln!("usage:");
            eprintln!("  aemu probe-so <lib.so>");
            eprintln!("  aemu probe-apk <app.apk>");
            eprintln!("  aemu plan-apk <app.apk>");
            eprintln!("  aemu imports-so <lib.so> [--limit N|--all]");
            eprintln!("  aemu imports-apk <app.apk> [--limit N|--all]");
            eprintln!("  aemu link-apk <app.apk> [--abi ABI] [--limit N|--all]");
            eprintln!(
                "  aemu run-apk-native <app.apk> [--abi ABI] [--cpu-backend aemu|qemu-armv7a-tcg] [--steps N] [--launch] [--until-swap] [--gles-summary] [--sdl2] [--sdl2-live] [--sdl2-frames N] [--ws ADDR]"
            );
            eprintln!("  aemu qemu-tcg-smoke");
            eprintln!("  aemu cpu-compare-smoke");
            eprintln!("  aemu sdl2-shell [--frames N] [--width W] [--height H]");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_qemu_tcg_smoke() -> Result<(), String> {
    let exit = run_qemu_tcg_smoke_exit()?;
    println!("qemu-armv7a-tcg smoke exit: {}", exit.status_text);
    if exit.code != Some(42) {
        return Err(format!("expected exit code 42, got {:?}", exit.code));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_qemu_tcg_smoke_exit() -> Result<aemu::qemu_tcg::QemuTcgExit, String> {
    let runner = aemu::qemu_tcg::QemuArmTcgRunner::probe().map_err(|err| err.to_string())?;
    runner
        .run_linux_exit_asm(
            aemu::qemu_tcg::QemuTcgArch::Armv7a,
            &aemu::qemu_tcg::armv7a_smoke_program(),
        )
        .map_err(|err| err.to_string())
}

#[cfg(target_arch = "wasm32")]
fn run_qemu_tcg_smoke() -> Result<(), String> {
    Err("qemu-tcg-smoke is only available on native hosts".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn run_cpu_compare_smoke() -> Result<(), String> {
    let qemu_exit = run_qemu_tcg_smoke_exit()?
        .code
        .ok_or_else(|| "qemu-arm exited without a numeric code".to_string())?;
    let aemu_exit = run_aemu_interpreter_smoke_exit()?;
    println!("cpu compare smoke: qemu={qemu_exit} aemu={aemu_exit}");
    if qemu_exit != aemu_exit {
        return Err(format!(
            "backend mismatch for ARMv7-A smoke program: qemu={qemu_exit} aemu={aemu_exit}"
        ));
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn run_cpu_compare_smoke() -> Result<(), String> {
    Err("cpu-compare-smoke is only available on native hosts".to_string())
}

fn run_aemu_interpreter_smoke_exit() -> Result<i32, String> {
    let mut cpu = Cpu::new();
    let mut memory = VecMemory::new(0x1000, 0x1000);
    memory
        .load_arm_words(0x1000, aemu::qemu_tcg::armv7a_smoke_arm_words())
        .map_err(|err| err.to_string())?;
    cpu.set_isa(Isa::Arm);
    cpu.branch_exchange(0x1000);
    for _ in 0..8 {
        match cpu.step(&mut memory) {
            Ok(()) => {}
            Err(Trap::SoftwareInterrupt { .. }) => return Ok((cpu.reg(0) & 0xff) as i32),
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("AEMU interpreter smoke did not reach SVC".to_string())
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

fn imports_so(path: &Path, limit: Option<usize>) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("failed to read ELF: {err}"))?;
    let info = aemu::elf_dynamic::parse_elf_dynamic_bytes(path.to_path_buf(), &bytes)
        .map_err(|err| format!("failed to parse dynamic metadata: {err}"))?;
    print_dynamic_info(&info, limit);
    Ok(())
}

fn imports_apk(path: &Path, limit: Option<usize>) -> Result<(), String> {
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
        let extracted = aemu::zip_probe::extract_parsed_zip_entry(&bytes, entry)
            .map_err(|err| format!("failed to extract {}: {err}", entry.name))?;
        match aemu::elf_dynamic::parse_elf_dynamic_bytes(
            PathBuf::from(format!("{}!{}", path.display(), entry.name)),
            &extracted,
        ) {
            Ok(info) => print_dynamic_info(&info, limit),
            Err(err) => println!("  dynamic parse failed: {err}"),
        }
    }
    Ok(())
}

fn parse_import_limit(args: Vec<String>) -> Result<Option<usize>, String> {
    let mut limit = Some(40usize);
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--all" => limit = None,
            "--limit" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--limit needs a numeric value".to_string())?;
                limit = Some(
                    value
                        .parse()
                        .map_err(|_| format!("invalid --limit value: {value}"))?,
                );
            }
            _ => return Err(format!("unknown imports argument: {arg}")),
        }
    }
    Ok(limit)
}

fn print_dynamic_info(info: &aemu::elf_dynamic::ElfDynamicInfo, limit: Option<usize>) {
    println!("path: {}", info.path.display());
    if info.needed.is_empty() {
        println!("needed: <none>");
    } else {
        println!("needed:");
        for needed in &info.needed {
            println!("  {needed}");
        }
    }

    match info.dynsym {
        Some(dynsym) => println!(
            "dynsym: addr {:#010x}, entry_size {}",
            dynsym.addr,
            dynsym
                .entry_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
        None => println!("dynsym: <none>"),
    }
    print_relocations(&info.relocations);
    println!("relocation_entries: {}", info.relocation_entries.len());
    if let Some(init) = info.init {
        println!("init: {init:#010x}");
    }
    if let Some(init_array) = info.init_array {
        println!(
            "init_array: addr {:#010x}, size {} bytes",
            init_array.addr, init_array.size
        );
    }

    println!("imports: {}", info.imports.len());
    let shown = limit.unwrap_or(info.imports.len()).min(info.imports.len());
    for import in info.imports.iter().take(shown) {
        println!("  {} [{} {}]", import.name, import.binding, import.kind);
    }
    if shown < info.imports.len() {
        println!(
            "  ... {} more imports (use --all or --limit N)",
            info.imports.len() - shown
        );
    }
}

fn print_relocations(relocations: &aemu::elf_dynamic::ElfRelocationInfo) {
    match relocations.rel {
        Some(rel) => println!(
            "relocations: rel addr {:#010x}, size {} bytes, entry_size {}",
            rel.addr,
            rel.size,
            relocations
                .rel_entry_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
        None => println!("relocations: rel <none>"),
    }
    match relocations.plt_rel {
        Some(plt) => println!(
            "plt_relocations: addr {:#010x}, size {} bytes, kind {}",
            plt.addr,
            plt.size,
            relocations
                .plt_rel_kind
                .map(|kind| kind.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
        None => println!("plt_relocations: <none>"),
    }
}

fn parse_link_apk_args(
    args: Vec<String>,
) -> Result<
    (
        PathBuf,
        aemu::native_loader::NativeLoadConfig,
        Option<usize>,
    ),
    String,
> {
    let mut iter = args.into_iter();
    let Some(path) = iter.next() else {
        return Err("missing APK path".to_string());
    };

    let mut config = aemu::native_loader::NativeLoadConfig::default();
    let mut limit = Some(40usize);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--abi" => {
                config.abi = iter
                    .next()
                    .ok_or_else(|| "--abi needs an ABI value".to_string())?;
            }
            "--all" => limit = None,
            "--limit" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--limit needs a numeric value".to_string())?;
                limit = Some(
                    value
                        .parse()
                        .map_err(|_| format!("invalid --limit value: {value}"))?,
                );
            }
            _ => return Err(format!("unknown link-apk argument: {arg}")),
        }
    }

    Ok((PathBuf::from(path), config, limit))
}

fn print_native_link_report(report: &aemu::native_loader::NativeLinkReport, limit: Option<usize>) {
    println!("apk: {}", report.apk_path.display());
    println!("abi: {}", report.abi);
    println!(
        "result: {}",
        if report.is_linked() {
            "loaded and relocated"
        } else {
            "loaded with unresolved link work"
        }
    );
    println!("addressing: 1:1 guest virtual addresses");
    println!("objects: {}", report.objects.len());
    for object in &report.objects {
        println!(
            "  {}: load_bias {:#010x}, mapped {:#010x}+{:#x}, entry {:#010x}, relocations {}",
            object.library_name,
            object.load_bias,
            object.memory_base,
            object.memory_size,
            object.entry,
            object.relocation_count
        );
        if !object.needed.is_empty() {
            println!("    needed: {}", object.needed.join(", "));
        }
        if let Some(init) = object.init {
            println!("    init: {init:#010x}");
        }
        if let Some(init_array) = object.init_array {
            println!(
                "    init_array: {:#010x}+{:#x}",
                init_array.addr, init_array.size
            );
        }
    }

    println!("native exports: {}", report.global_symbols.len());
    println!("HLE reserved symbols: {}", report.hle_symbols.len());
    print_hle_symbols(report, limit);
    println!("resolved imports: {}", report.resolved_imports.len());
    println!("unresolved imports: {}", report.unresolved_imports.len());
    print_unresolved_imports(report, limit);
    if !report.relocation_errors.is_empty() {
        println!("relocation errors: {}", report.relocation_errors.len());
        for error in &report.relocation_errors {
            println!("  {}: {}", error.library_name, error.error);
        }
    }
}

fn print_hle_symbols(report: &aemu::native_loader::NativeLinkReport, limit: Option<usize>) {
    let shown = limit
        .unwrap_or(report.hle_symbols.len())
        .min(report.hle_symbols.len());
    for symbol in report.hle_symbols.iter().take(shown) {
        println!(
            "  {:#010x} {} [{} {} {}]",
            symbol.address, symbol.name, symbol.kind, symbol.shape, symbol.behavior
        );
    }
    if shown < report.hle_symbols.len() {
        println!(
            "  ... {} more HLE symbols (use --all or --limit N)",
            report.hle_symbols.len() - shown
        );
    }
}

fn print_unresolved_imports(report: &aemu::native_loader::NativeLinkReport, limit: Option<usize>) {
    let shown = limit
        .unwrap_or(report.unresolved_imports.len())
        .min(report.unresolved_imports.len());
    for import in report.unresolved_imports.iter().take(shown) {
        println!(
            "  {}: {} [{}]",
            import.library_name, import.name, import.binding
        );
    }
    if shown < report.unresolved_imports.len() {
        println!(
            "  ... {} more unresolved imports (use --all or --limit N)",
            report.unresolved_imports.len() - shown
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunApkNativeOptions {
    cpu_backend: aemu::native_runtime::NativeCpuBackendKind,
    launch: bool,
    until_swap: bool,
    gles_summary: bool,
    sdl2: bool,
    sdl2_live: bool,
    sdl2_frames: Option<u64>,
    sdl2_hold_ms: u64,
    ws_addr: Option<String>,
}

fn parse_run_apk_native_args(
    args: Vec<String>,
) -> Result<
    (
        PathBuf,
        aemu::native_loader::NativeLoadConfig,
        usize,
        RunApkNativeOptions,
    ),
    String,
> {
    let mut iter = args.into_iter();
    let Some(path) = iter.next() else {
        return Err("missing APK path".to_string());
    };

    let mut config = aemu::native_loader::NativeLoadConfig::default();
    let mut max_steps = 100_000usize;
    let mut max_steps_set = false;
    let mut options = RunApkNativeOptions {
        cpu_backend: aemu::native_runtime::NativeCpuBackendKind::AemuInterpreter,
        launch: false,
        until_swap: false,
        gles_summary: false,
        sdl2: false,
        sdl2_live: false,
        sdl2_frames: None,
        sdl2_hold_ms: 1000,
        ws_addr: None,
    };
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--abi" => {
                config.abi = iter
                    .next()
                    .ok_or_else(|| "--abi needs an ABI value".to_string())?;
            }
            "--cpu-backend" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--cpu-backend needs a backend value".to_string())?;
                options.cpu_backend = aemu::native_runtime::NativeCpuBackendKind::parse(&value)
                    .ok_or_else(|| {
                        format!(
                            "invalid --cpu-backend value: {value} (expected aemu or qemu-armv7a-tcg)"
                        )
                    })?;
            }
            "--steps" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--steps needs a numeric value".to_string())?;
                max_steps = value
                    .parse()
                    .map_err(|_| format!("invalid --steps value: {value}"))?;
                max_steps_set = true;
            }
            "--launch" => options.launch = true,
            "--until-swap" => {
                options.launch = true;
                options.until_swap = true;
            }
            "--gles-summary" => options.gles_summary = true,
            "--sdl2" => {
                options.launch = true;
                options.until_swap = true;
                options.sdl2 = true;
            }
            "--sdl2-live" => {
                options.launch = true;
                options.until_swap = true;
                options.sdl2_live = true;
            }
            "--sdl2-frames" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--sdl2-frames needs a numeric value".to_string())?;
                options.sdl2_frames = Some(
                    value
                        .parse()
                        .map_err(|_| format!("invalid --sdl2-frames value: {value}"))?,
                );
            }
            "--ws" => {
                options.launch = true;
                options.until_swap = true;
                options.sdl2_live = true;
                options.ws_addr = Some(
                    iter.next()
                        .ok_or_else(|| "--ws needs a listen address".to_string())?,
                );
            }
            "--sdl2-hold-ms" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--sdl2-hold-ms needs a numeric value".to_string())?;
                options.sdl2_hold_ms = value
                    .parse()
                    .map_err(|_| format!("invalid --sdl2-hold-ms value: {value}"))?;
            }
            _ => return Err(format!("unknown run-apk-native argument: {arg}")),
        }
    }
    if !max_steps_set && options.until_swap {
        max_steps = 300_000_000;
    }

    Ok((PathBuf::from(path), config, max_steps, options))
}

fn run_apk_native(
    path: &Path,
    config: &aemu::native_loader::NativeLoadConfig,
    max_steps: usize,
    options: RunApkNativeOptions,
) -> Result<(), String> {
    #[cfg(not(feature = "sdl2"))]
    if options.sdl2 || options.sdl2_live {
        return Err("--sdl2/--sdl2-live requires rebuilding with --features sdl2".to_string());
    }

    let report = aemu::native_loader::load_apk_native_libraries_with_config(path, config)
        .map_err(|err| format!("link failed: {err}"))?;
    if !report.is_linked() {
        return Err(format!(
            "link incomplete: {} unresolved imports, {} relocation errors",
            report.unresolved_imports.len(),
            report.relocation_errors.len()
        ));
    }
    let mut runtime = aemu::native_runtime::NativeRuntime::new_with_cpu_backend(
        report,
        aemu::native_runtime::NativeRuntimeConfig::default(),
        options.cpu_backend,
    )
    .map_err(|err| format!("runtime setup failed: {err}"))?;
    let constructors = runtime
        .constructors()
        .map_err(|err| format!("constructor scan failed: {err}"))?;
    println!("apk: {}", path.display());
    println!("abi: {}", config.abi);
    if options.sdl2_live {
        println!(
            "constructors: {} (addresses suppressed for SDL2 live)",
            constructors.len()
        );
    } else {
        println!("constructors: {}", constructors.len());
    }
    for constructor in constructors {
        if !options.sdl2_live {
            println!(
                "  {} {:?} {:#010x}",
                constructor.library_name, constructor.source, constructor.address
            );
        }
        runtime
            .run_function(constructor.address, max_steps)
            .map_err(|err| {
                format!(
                    "{} constructor {:#010x} failed: {err}",
                    constructor.library_name, constructor.address
                )
            })?;
    }
    println!("native constructors completed");
    if options.launch {
        run_native_activity_launch(&mut runtime, max_steps, options.until_swap)?;
    }
    if options.sdl2_live {
        runtime.hle.enable_cxx_string_recycling();
        replay_sdl2_live_gles_frames(
            &mut runtime,
            max_steps,
            options.sdl2_frames,
            options.ws_addr.as_deref(),
        )?;
    } else if options.gles_summary || options.sdl2 {
        let events = runtime.hle.take_gles_events();
        if options.gles_summary {
            print_gles_summary(&events);
        }
        if options.sdl2 {
            replay_sdl2_gles_events(&events, options.sdl2_hold_ms)?;
        }
    }
    Ok(())
}

fn print_gles_summary(events: &[aemu::hle_imports::GlesEvent]) {
    let mut counts = BTreeMap::new();
    let mut payload_bytes = 0usize;
    for event in events {
        *counts.entry(event.kind()).or_insert(0usize) += 1;
        payload_bytes += event.payload_len();
    }
    println!("gles events: {}", events.len());
    println!("gles payload bytes: {payload_bytes}");
    for (kind, count) in counts {
        println!("  {kind}: {count}");
    }
}

#[cfg(feature = "sdl2")]
fn replay_sdl2_gles_events(
    events: &[aemu::hle_imports::GlesEvent],
    hold_ms: u64,
) -> Result<(), String> {
    use std::thread;
    use std::time::{Duration, Instant};

    use aemu::host::{HostBackend, HostConfig, HostEvent, HostKey};

    let mut host = aemu::sdl_shell::Sdl2Host::new(&HostConfig::default())
        .map_err(|err| format!("SDL2 setup failed: {err}"))?;
    println!("sdl2: replaying {} GLES events", events.len());
    host.replay_gles_events(events)
        .map_err(|err| format!("SDL2 GLES replay failed: {err}"))?;
    let stats = host.replay_stats();
    println!(
        "sdl2: submitted draws arrays={} elements={} skipped_client_attrib={} skipped_missing_indices={}",
        stats.draw_arrays,
        stats.draw_elements,
        stats.skipped_client_attrib_draws,
        stats.skipped_missing_index_draws
    );
    println!(
        "sdl2: readback {}x{} nonzero_rgb_pixels={} nonzero_alpha_pixels={}",
        stats.readback_width,
        stats.readback_height,
        stats.readback_nonzero_rgb_pixels,
        stats.readback_nonzero_alpha_pixels
    );
    println!(
        "sdl2: gl_errors count={} first_event={} first_kind={} first_code=0x{:04x}",
        stats.gl_error_count,
        stats.first_gl_error_event_index,
        stats.first_gl_error_event_kind.unwrap_or("none"),
        stats.first_gl_error_code
    );
    if stats.gl_error_count > 0 {
        if let Some(event) = events.get(stats.first_gl_error_event_index) {
            println!("sdl2: first_gl_error_event {}", gles_event_brief(event));
        }
    }

    let deadline = Instant::now() + Duration::from_millis(hold_ms);
    while Instant::now() < deadline {
        for event in host
            .poll_events()
            .map_err(|err| format!("SDL2 event poll failed: {err}"))?
        {
            match event {
                HostEvent::Quit
                | HostEvent::Key {
                    key: HostKey::Escape,
                    pressed: true,
                    ..
                } => return Ok(()),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}

#[cfg(feature = "sdl2")]
fn gles_event_brief(event: &aemu::hle_imports::GlesEvent) -> String {
    use aemu::hle_imports::GlesEvent;

    match event {
        GlesEvent::BufferData {
            target,
            size,
            usage,
            payload,
            ..
        } => format!(
            "BufferData target=0x{target:04x} size={size} usage=0x{usage:04x} payload_len={}",
            payload.as_ref().map_or(0, Vec::len)
        ),
        GlesEvent::BufferSubData {
            target,
            offset,
            size,
            payload,
            ..
        } => format!(
            "BufferSubData target=0x{target:04x} offset={offset} size={size} payload_len={}",
            payload.as_ref().map_or(0, Vec::len)
        ),
        GlesEvent::TexImage2D {
            target,
            level,
            internal_format,
            width,
            height,
            format,
            ty,
            payload,
            ..
        } => format!(
            "TexImage2D target=0x{target:04x} level={level} internal_format=0x{internal_format:04x} size={width}x{height} format=0x{format:04x} type=0x{ty:04x} payload_len={}",
            payload.as_ref().map_or(0, Vec::len)
        ),
        GlesEvent::TexSubImage2D {
            target,
            level,
            xoffset,
            yoffset,
            width,
            height,
            format,
            ty,
            payload,
            ..
        } => format!(
            "TexSubImage2D target=0x{target:04x} level={level} offset={xoffset},{yoffset} size={width}x{height} format=0x{format:04x} type=0x{ty:04x} payload_len={}",
            payload.as_ref().map_or(0, Vec::len)
        ),
        GlesEvent::DrawElements {
            mode,
            count,
            ty,
            indices,
            index_payload,
            client_attribs,
        } => format!(
            "DrawElements mode=0x{mode:04x} count={count} type=0x{ty:04x} indices=0x{indices:08x} index_payload_len={} client_attribs={}",
            index_payload.as_ref().map_or(0, Vec::len),
            client_attribs.len()
        ),
        _ => format!("{event:?}"),
    }
}

#[cfg(not(feature = "sdl2"))]
fn replay_sdl2_gles_events(
    _events: &[aemu::hle_imports::GlesEvent],
    _hold_ms: u64,
) -> Result<(), String> {
    Err("--sdl2 requires rebuilding with --features sdl2".to_string())
}

#[cfg(feature = "sdl2")]
fn replay_sdl2_live_gles_frames(
    runtime: &mut aemu::native_runtime::NativeRuntime,
    max_steps_per_frame: usize,
    max_frames: Option<u64>,
    ws_addr: Option<&str>,
) -> Result<(), String> {
    use aemu::hle_imports::GlesEvent;
    use aemu::host::{HostBackend, HostConfig, HostEvent, HostKey};
    use aemu::ws_harness::{WsCommand, WsHarness};
    use base64::Engine;
    use serde_json::json;

    let mut host = aemu::sdl_shell::Sdl2Host::new(&HostConfig::default())
        .map_err(|err| format!("SDL2 setup failed: {err}"))?;
    let ws = ws_addr
        .map(|addr| WsHarness::start(addr).map_err(|err| format!("WebSocket setup failed: {err}")))
        .transpose()?;
    if let Some(ws) = ws.as_ref() {
        println!("sdl2-live: websocket ws://{}", ws.local_addr());
    }
    let mut frames = 0_u64;
    let mut total_events = 0_usize;
    let mut total_payload_bytes = 0_usize;

    println!(
        "sdl2-live: started max_frames={} max_steps_per_frame={}",
        max_frames
            .map(|frames| frames.to_string())
            .unwrap_or_else(|| "unlimited".to_string()),
        max_steps_per_frame
    );

    loop {
        let events = runtime.hle.take_gles_events();
        let frame_swaps = events
            .iter()
            .filter(|event| matches!(event, GlesEvent::SwapBuffers { .. }))
            .count() as u64;
        let payload_bytes = events
            .iter()
            .map(|event| event.payload_len())
            .sum::<usize>();
        total_events += events.len();
        total_payload_bytes += payload_bytes;
        if !events.is_empty() {
            host.replay_gles_events(&events)
                .map_err(|err| format!("SDL2 GLES replay failed: {err}"))?;
        }
        frames += frame_swaps;

        if frame_swaps != 0
            && (frames <= 5 || frames % 60 == 0 || max_frames.is_some_and(|limit| frames >= limit))
        {
            let stats = host.replay_stats();
            println!(
                "sdl2-live: frame={} events={} payload={} draws arrays={} elements={} skipped_client_attrib={} skipped_missing_indices={} readback={}x{} rgb={} alpha={} gl_errors={}",
                frames,
                events.len(),
                payload_bytes,
                stats.draw_arrays,
                stats.draw_elements,
                stats.skipped_client_attrib_draws,
                stats.skipped_missing_index_draws,
                stats.readback_width,
                stats.readback_height,
                stats.readback_nonzero_rgb_pixels,
                stats.readback_nonzero_alpha_pixels,
                stats.gl_error_count
            );
        }

        for event in host
            .poll_events()
            .map_err(|err| format!("SDL2 event poll failed: {err}"))?
        {
            match event {
                HostEvent::Quit
                | HostEvent::Key {
                    key: HostKey::Escape,
                    pressed: true,
                    ..
                } => {
                    println!(
                        "sdl2-live: stopped by host event frames={} events={} payload={}",
                        frames, total_events, total_payload_bytes
                    );
                    return Ok(());
                }
                HostEvent::Pointer {
                    id,
                    phase,
                    x,
                    y,
                    pressure,
                } => {
                    runtime
                        .hle
                        .push_pointer_event(id, hle_pointer_phase(phase), x, y, pressure);
                }
                _ => {}
            }
        }

        if let Some(ws) = ws.as_ref() {
            while let Some(request) = ws.try_recv() {
                match request.command {
                    WsCommand::Debug => {
                        let stats = host.replay_stats();
                        request.respond_ok(json!({
                            "ok": true,
                            "frames": frames,
                            "total_events": total_events,
                            "total_payload_bytes": total_payload_bytes,
                            "draw_arrays": stats.draw_arrays,
                            "draw_elements": stats.draw_elements,
                            "skipped_client_attrib_draws": stats.skipped_client_attrib_draws,
                            "skipped_missing_index_draws": stats.skipped_missing_index_draws,
                            "readback_width": stats.readback_width,
                            "readback_height": stats.readback_height,
                            "readback_nonzero_rgb_pixels": stats.readback_nonzero_rgb_pixels,
                            "readback_nonzero_alpha_pixels": stats.readback_nonzero_alpha_pixels,
                            "gl_error_count": stats.gl_error_count,
                            "first_gl_error_event_index": stats.first_gl_error_event_index,
                            "first_gl_error_event_kind": stats.first_gl_error_event_kind,
                            "first_gl_error_code": stats.first_gl_error_code,
                        }));
                    }
                    WsCommand::Screenshot => match host.capture_framebuffer_rgb() {
                        Ok(capture) => match aemu::png_util::encode_rgb_png(
                            capture.width,
                            capture.height,
                            &capture.rgb,
                        ) {
                            Ok(png) => {
                                let data_base64 =
                                    base64::engine::general_purpose::STANDARD.encode(png);
                                request.respond_ok(json!({
                                    "ok": true,
                                    "format": "png",
                                    "width": capture.width,
                                    "height": capture.height,
                                    "data_base64": data_base64,
                                }));
                            }
                            Err(err) => request
                                .respond_error(format!("screenshot PNG encode failed: {err}")),
                        },
                        Err(err) => request.respond_error(format!("screenshot failed: {err}")),
                    },
                    WsCommand::Pointer {
                        id,
                        phase,
                        x,
                        y,
                        pressure,
                    } => {
                        runtime
                            .hle
                            .push_pointer_event(id, ws_pointer_phase(phase), x, y, pressure);
                        request.respond_ok(json!({
                            "ok": true,
                            "id": id,
                            "phase": format!("{phase:?}").to_lowercase(),
                            "x": x,
                            "y": y,
                            "pressure": pressure,
                        }));
                    }
                }
            }
        }

        if max_frames.is_some_and(|limit| frames >= limit) {
            println!(
                "sdl2-live: reached frame limit frames={} events={} payload={}",
                frames, total_events, total_payload_bytes
            );
            return Ok(());
        }

        match runtime
            .continue_until_hle(max_steps_per_frame, Some("eglSwapBuffers"))
            .map_err(|err| format!("android_main continuation failed: {err}"))?
        {
            aemu::native_runtime::NativeRuntimeFunctionExit::HleCall { step, .. } => {
                if frames < 3 || frames % 60 == 0 {
                    println!("sdl2-live: next eglSwapBuffers after {step} guest steps");
                }
            }
            aemu::native_runtime::NativeRuntimeFunctionExit::Returned => {
                return Err("android_main returned during SDL2 live loop".to_string());
            }
        }
    }
}

#[cfg(feature = "sdl2")]
fn hle_pointer_phase(phase: aemu::host::PointerPhase) -> aemu::hle_imports::HlePointerPhase {
    match phase {
        aemu::host::PointerPhase::Down => aemu::hle_imports::HlePointerPhase::Down,
        aemu::host::PointerPhase::Up => aemu::hle_imports::HlePointerPhase::Up,
        aemu::host::PointerPhase::Move => aemu::hle_imports::HlePointerPhase::Move,
    }
}

#[cfg(feature = "sdl2")]
fn ws_pointer_phase(phase: aemu::ws_harness::WsPointerPhase) -> aemu::hle_imports::HlePointerPhase {
    match phase {
        aemu::ws_harness::WsPointerPhase::Down => aemu::hle_imports::HlePointerPhase::Down,
        aemu::ws_harness::WsPointerPhase::Up => aemu::hle_imports::HlePointerPhase::Up,
        aemu::ws_harness::WsPointerPhase::Move => aemu::hle_imports::HlePointerPhase::Move,
    }
}

#[cfg(not(feature = "sdl2"))]
fn replay_sdl2_live_gles_frames(
    _runtime: &mut aemu::native_runtime::NativeRuntime,
    _max_steps_per_frame: usize,
    _max_frames: Option<u64>,
    _ws_addr: Option<&str>,
) -> Result<(), String> {
    Err("--sdl2-live requires rebuilding with --features sdl2".to_string())
}

fn run_native_activity_launch(
    runtime: &mut aemu::native_runtime::NativeRuntime,
    max_steps: usize,
    until_swap: bool,
) -> Result<(), String> {
    const ACTIVITY_LIBRARY: &str = "libminecraftpe.so";
    let on_create = runtime
        .symbol_address_in_library(ACTIVITY_LIBRARY, "ANativeActivity_onCreate")
        .or_else(|| runtime.symbol_address("ANativeActivity_onCreate"))
        .ok_or_else(|| "missing ANativeActivity_onCreate export".to_string())?;
    let harness = runtime
        .prepare_native_activity()
        .map_err(|err| format!("native activity harness setup failed: {err}"))?;

    let jni_on_loads: Vec<(String, u32)> = runtime
        .link
        .objects
        .iter()
        .filter_map(|object| {
            object
                .defined_symbols
                .iter()
                .find(|symbol| symbol.name == "JNI_OnLoad")
                .map(|symbol| (object.library_name.clone(), symbol.address))
        })
        .collect();
    for (library_name, jni_on_load) in jni_on_loads {
        println!(
            "launch: {library_name} JNI_OnLoad {jni_on_load:#010x} java_vm {:#010x}",
            harness.java_vm
        );
        runtime
            .run_function_with_args(jni_on_load, &[harness.java_vm, 0], max_steps)
            .map_err(|err| format!("{library_name} JNI_OnLoad failed: {err}"))?;
    }

    if let Some(native_register_this) = runtime.symbol_address_in_library(
        ACTIVITY_LIBRARY,
        "Java_com_mojang_minecraftpe_MainActivity_nativeRegisterThis",
    ) {
        println!(
            "launch: nativeRegisterThis {native_register_this:#010x} env {:#010x}",
            harness.jni_env
        );
        runtime
            .run_function_with_args(
                native_register_this,
                &[harness.jni_env, harness.activity_class],
                max_steps,
            )
            .map_err(|err| format!("nativeRegisterThis failed: {err}"))?;
    }

    println!(
        "launch: ANativeActivity_onCreate {on_create:#010x} activity {:#010x}",
        harness.activity
    );
    runtime
        .run_function_with_args(on_create, &[harness.activity, 0, 0], max_steps)
        .map_err(|err| format!("ANativeActivity_onCreate failed: {err}"))?;

    let mut app = runtime
        .link
        .memory
        .load32(harness.activity.wrapping_add(0x1c))
        .map_err(|err| format!("failed to read ANativeActivity.instance: {err}"))?;
    if app == 0 {
        app = runtime
            .prepare_android_app(harness)
            .map_err(|err| format!("android_app harness setup failed: {err}"))?;
    } else {
        runtime
            .populate_android_app(app, harness)
            .map_err(|err| format!("android_app harness patch failed: {err}"))?;
    }

    let android_main = runtime
        .symbol_address_in_library(ACTIVITY_LIBRARY, "android_main")
        .or_else(|| runtime.symbol_address("android_main"))
        .ok_or_else(|| "missing android_main export".to_string())?;
    println!("launch: android_main {android_main:#010x} android_app {app:#010x}");
    if until_swap {
        let outcome = runtime
            .run_function_with_args_until_hle(
                android_main,
                &[app],
                max_steps,
                Some("eglSwapBuffers"),
            )
            .map_err(|err| format!("android_main failed: {err}"))?;
        return match outcome {
            aemu::native_runtime::NativeRuntimeFunctionExit::HleCall { name, step, .. } => {
                println!("native activity reached {name} at step {step}");
                Ok(())
            }
            aemu::native_runtime::NativeRuntimeFunctionExit::Returned => {
                Err("android_main returned before eglSwapBuffers".to_string())
            }
        };
    }
    runtime
        .run_function_with_args(android_main, &[app], max_steps)
        .map_err(|err| format!("android_main failed: {err}"))?;
    println!("native activity launch returned");
    Ok(())
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
