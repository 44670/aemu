use std::env;
use std::path::{Path, PathBuf};

use aemu::armv6::Memory;

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
            let (path, config, max_steps, launch) = match parse_run_apk_native_args(args.collect())
            {
                Ok(parsed) => parsed,
                Err(err) => {
                    eprintln!("{err}");
                    eprintln!(
                        "usage: aemu run-apk-native <app.apk> [--abi ABI] [--steps N] [--launch]"
                    );
                    std::process::exit(2);
                }
            };
            if let Err(err) = run_apk_native(&path, &config, max_steps, launch) {
                eprintln!("native run failed: {err}");
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
            eprintln!("  aemu run-apk-native <app.apk> [--abi ABI] [--steps N] [--launch]");
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

fn parse_run_apk_native_args(
    args: Vec<String>,
) -> Result<(PathBuf, aemu::native_loader::NativeLoadConfig, usize, bool), String> {
    let mut iter = args.into_iter();
    let Some(path) = iter.next() else {
        return Err("missing APK path".to_string());
    };

    let mut config = aemu::native_loader::NativeLoadConfig::default();
    let mut max_steps = 100_000usize;
    let mut launch = false;
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--abi" => {
                config.abi = iter
                    .next()
                    .ok_or_else(|| "--abi needs an ABI value".to_string())?;
            }
            "--steps" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--steps needs a numeric value".to_string())?;
                max_steps = value
                    .parse()
                    .map_err(|_| format!("invalid --steps value: {value}"))?;
            }
            "--launch" => launch = true,
            _ => return Err(format!("unknown run-apk-native argument: {arg}")),
        }
    }

    Ok((PathBuf::from(path), config, max_steps, launch))
}

fn run_apk_native(
    path: &Path,
    config: &aemu::native_loader::NativeLoadConfig,
    max_steps: usize,
    launch: bool,
) -> Result<(), String> {
    let report = aemu::native_loader::load_apk_native_libraries_with_config(path, config)
        .map_err(|err| format!("link failed: {err}"))?;
    if !report.is_linked() {
        return Err(format!(
            "link incomplete: {} unresolved imports, {} relocation errors",
            report.unresolved_imports.len(),
            report.relocation_errors.len()
        ));
    }
    let mut runtime = aemu::native_runtime::NativeRuntime::new(
        report,
        aemu::native_runtime::NativeRuntimeConfig::default(),
    )
    .map_err(|err| format!("runtime setup failed: {err}"))?;
    let constructors = runtime
        .constructors()
        .map_err(|err| format!("constructor scan failed: {err}"))?;
    println!("apk: {}", path.display());
    println!("abi: {}", config.abi);
    println!("constructors: {}", constructors.len());
    for constructor in constructors {
        println!(
            "  {} {:?} {:#010x}",
            constructor.library_name, constructor.source, constructor.address
        );
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
    if launch {
        run_native_activity_launch(&mut runtime, max_steps)?;
    }
    Ok(())
}

fn run_native_activity_launch(
    runtime: &mut aemu::native_runtime::NativeRuntime,
    max_steps: usize,
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
