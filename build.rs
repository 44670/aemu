use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var_os("CARGO_FEATURE_DYNARMIC").is_none() {
        return;
    }

    let target_family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    if target_family == "wasm" {
        panic!("the dynarmic feature is native-only and is not supported for wasm targets");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dynarmic_src = env::var_os("AEMU_DYNARMIC_SRC")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("../aemu-refs/dynarmic"));
    if !dynarmic_src.join("CMakeLists.txt").exists() {
        panic!(
            "dynarmic source checkout not found at {}; set AEMU_DYNARMIC_SRC",
            dynarmic_src.display()
        );
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let build_dir = out_dir.join("dynarmic-build");

    run(Command::new("cmake")
        .arg("-S")
        .arg(&dynarmic_src)
        .arg("-B")
        .arg(&build_dir)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg("-DCMAKE_POSITION_INDEPENDENT_CODE=ON")
        .arg("-DDYNARMIC_TESTS=OFF")
        .arg("-DDYNARMIC_USE_BUNDLED_EXTERNALS=ON")
        .arg("-DDYNARMIC_FRONTENDS=A32")
        .arg("-DDYNARMIC_USE_LLVM=OFF")
        .arg("-DDYNARMIC_USE_PRECOMPILED_HEADERS=OFF")
        .arg("-DDYNARMIC_WARNINGS_AS_ERRORS=OFF"));
    run(Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("dynarmic"));

    let shim_obj = out_dir.join("aemu_dynarmic_shim.o");
    run(
        Command::new(env::var("CXX").unwrap_or_else(|_| "c++".to_string()))
            .arg("-std=c++20")
            .arg("-O2")
            .arg("-fPIC")
            .arg("-I")
            .arg(dynarmic_src.join("src"))
            .arg("-I")
            .arg(dynarmic_src.join("externals/mcl/include"))
            .arg("-I")
            .arg(dynarmic_src.join("externals/fmt/include"))
            .arg("-I")
            .arg(dynarmic_src.join("externals/xbyak/xbyak"))
            .arg("-I")
            .arg(dynarmic_src.join("externals/zydis/include"))
            .arg("-I")
            .arg(dynarmic_src.join("externals/zydis/zycore/include"))
            .arg("-c")
            .arg(manifest_dir.join("src/dynarmic_shim.cpp"))
            .arg("-o")
            .arg(&shim_obj),
    );

    let shim_lib = out_dir.join("libaemu_dynarmic_shim.a");
    run(
        Command::new(env::var("AR").unwrap_or_else(|_| "ar".to_string()))
            .arg("crus")
            .arg(&shim_lib)
            .arg(&shim_obj),
    );

    println!("cargo:rerun-if-changed=src/dynarmic_shim.cpp");
    println!("cargo:rerun-if-env-changed=AEMU_DYNARMIC_SRC");
    println!("cargo:rerun-if-env-changed=CXX");
    println!("cargo:rerun-if-env-changed=AR");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.join("src/dynarmic").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.join("externals/mcl/src").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.join("externals/fmt").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.join("externals/zydis").display()
    );
    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.join("externals/zydis/zycore").display()
    );
    println!("cargo:rustc-link-lib=static=aemu_dynarmic_shim");
    println!("cargo:rustc-link-lib=static=dynarmic");
    println!("cargo:rustc-link-lib=static=mcl");
    println!("cargo:rustc-link-lib=static=fmt");
    println!("cargo:rustc-link-lib=static=Zydis");
    println!("cargo:rustc-link-lib=static=Zycore");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=pthread");
    println!("cargo:rustc-link-lib=dylib=dl");
}

fn run(command: &mut Command) {
    let status = command
        .status()
        .unwrap_or_else(|err| panic!("failed to run {command:?}: {err}"));
    if !status.success() {
        panic!("{command:?} failed with {status}");
    }
}
