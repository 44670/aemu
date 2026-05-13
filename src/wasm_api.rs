use std::path::PathBuf;

use js_sys::{Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

use crate::armv6::Memory;
use crate::host::{GraphicsApi, HostBackend};
use crate::native_loader::{NativeLoadConfig, load_apk_native_libraries_bytes};
use crate::native_runtime::{NativeRuntime, NativeRuntimeConfig, NativeRuntimeFunctionExit};
use crate::wasm_webgl::BrowserWebGlHost;

const DEFAULT_BROWSER_APK_NAME: &str = "browser.apk";
const DEFAULT_MCPE_ABI: &str = "armeabi-v7a";
const DEFAULT_MCPE_FIRST_FRAME_STEPS: usize = 300_000_000;
const MCPE_LIBRARY: &str = "libminecraftpe.so";

#[wasm_bindgen(js_name = runMcpeFirstFrame)]
pub fn run_mcpe_first_frame(
    apk_bytes: &Uint8Array,
    abi: &str,
    canvas_id: &str,
    max_steps: u32,
) -> Result<JsValue, JsValue> {
    let bytes = apk_bytes.to_vec();
    let steps = if max_steps == 0 {
        DEFAULT_MCPE_FIRST_FRAME_STEPS
    } else {
        max_steps as usize
    };
    let abi = if abi.is_empty() {
        DEFAULT_MCPE_ABI
    } else {
        abi
    };
    let canvas_id = if canvas_id.is_empty() {
        crate::wasm_webgl::DEFAULT_CANVAS_ID
    } else {
        canvas_id
    };
    run_mcpe_first_frame_impl(bytes, abi, canvas_id, steps).map_err(|err| JsValue::from_str(&err))
}

fn run_mcpe_first_frame_impl(
    apk_bytes: Vec<u8>,
    abi: &str,
    canvas_id: &str,
    max_steps: usize,
) -> Result<JsValue, String> {
    let config = NativeLoadConfig {
        abi: abi.to_string(),
        ..NativeLoadConfig::default()
    };
    let report = load_apk_native_libraries_bytes(
        PathBuf::from(DEFAULT_BROWSER_APK_NAME),
        &apk_bytes,
        &config,
    )
    .map_err(|err| format!("link failed: {err}"))?;
    if !report.is_linked() {
        return Err(format!(
            "link incomplete: {} unresolved imports, {} relocation errors",
            report.unresolved_imports.len(),
            report.relocation_errors.len()
        ));
    }

    let mut runtime = NativeRuntime::new(report, NativeRuntimeConfig::default())
        .map_err(|err| format!("runtime setup failed: {err}"))?;
    runtime.hle.set_apk_bytes(apk_bytes);

    let constructors = runtime
        .constructors()
        .map_err(|err| format!("constructor scan failed: {err}"))?;
    let constructor_count = constructors.len();
    for constructor in constructors {
        runtime
            .run_function(constructor.address, max_steps)
            .map_err(|err| {
                format!(
                    "{} constructor {:#010x} failed: {err}",
                    constructor.library_name, constructor.address
                )
            })?;
    }

    let swap_step = run_native_activity_until_swap(&mut runtime, max_steps)?;
    let events = runtime.hle.take_gles_events();
    let gles_event_count = events.len();
    let gles_payload_bytes = events
        .iter()
        .map(|event| event.payload_len())
        .sum::<usize>();

    let mut host = BrowserWebGlHost::from_canvas_id(canvas_id, GraphicsApi::WebGl1)
        .map_err(|err| format!("WebGL setup failed: {err}"))?;
    host.replay_gles_events(&events)
        .map_err(|err| format!("WebGL replay failed: {err}"))?;
    let stats = host.replay_stats();

    let out = Object::new();
    set_prop(&out, "abi", JsValue::from_str(abi))?;
    set_prop(&out, "api", JsValue::from_str("webgl1"))?;
    set_prop(&out, "constructorCount", usize_value(constructor_count))?;
    set_prop(&out, "swapStep", usize_value(swap_step))?;
    set_prop(&out, "glesEvents", usize_value(gles_event_count))?;
    set_prop(&out, "glesPayloadBytes", usize_value(gles_payload_bytes))?;
    set_prop(&out, "drawArrays", usize_value(stats.draw_arrays))?;
    set_prop(&out, "drawElements", usize_value(stats.draw_elements))?;
    set_prop(
        &out,
        "skippedClientAttribDraws",
        usize_value(stats.skipped_client_attrib_draws),
    )?;
    set_prop(
        &out,
        "skippedMissingIndexDraws",
        usize_value(stats.skipped_missing_index_draws),
    )?;
    set_prop(
        &out,
        "readbackWidth",
        JsValue::from_f64(f64::from(stats.readback_width)),
    )?;
    set_prop(
        &out,
        "readbackHeight",
        JsValue::from_f64(f64::from(stats.readback_height)),
    )?;
    set_prop(
        &out,
        "readbackNonzeroRgbPixels",
        usize_value(stats.readback_nonzero_rgb_pixels),
    )?;
    set_prop(
        &out,
        "readbackNonzeroAlphaPixels",
        usize_value(stats.readback_nonzero_alpha_pixels),
    )?;
    set_prop(&out, "glErrorCount", usize_value(stats.gl_error_count))?;
    set_prop(
        &out,
        "firstGlErrorEventIndex",
        usize_value(stats.first_gl_error_event_index),
    )?;
    set_prop(
        &out,
        "firstGlErrorEventKind",
        stats
            .first_gl_error_event_kind
            .map(JsValue::from_str)
            .unwrap_or(JsValue::NULL),
    )?;
    set_prop(
        &out,
        "firstGlErrorCode",
        JsValue::from_f64(f64::from(stats.first_gl_error_code)),
    )?;
    Ok(out.into())
}

fn run_native_activity_until_swap(
    runtime: &mut NativeRuntime,
    max_steps: usize,
) -> Result<usize, String> {
    let on_create = runtime
        .symbol_address_in_library(MCPE_LIBRARY, "ANativeActivity_onCreate")
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
        runtime
            .run_function_with_args(jni_on_load, &[harness.java_vm, 0], max_steps)
            .map_err(|err| format!("{library_name} JNI_OnLoad failed: {err}"))?;
    }

    if let Some(native_register_this) = runtime.symbol_address_in_library(
        MCPE_LIBRARY,
        "Java_com_mojang_minecraftpe_MainActivity_nativeRegisterThis",
    ) {
        runtime
            .run_function_with_args(
                native_register_this,
                &[harness.jni_env, harness.activity_class],
                max_steps,
            )
            .map_err(|err| format!("nativeRegisterThis failed: {err}"))?;
    }

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
        .symbol_address_in_library(MCPE_LIBRARY, "android_main")
        .or_else(|| runtime.symbol_address("android_main"))
        .ok_or_else(|| "missing android_main export".to_string())?;
    match runtime
        .run_function_with_args_until_hle(android_main, &[app], max_steps, Some("eglSwapBuffers"))
        .map_err(|err| format!("android_main failed: {err}"))?
    {
        NativeRuntimeFunctionExit::HleCall { step, .. } => Ok(step),
        NativeRuntimeFunctionExit::Returned => {
            Err("android_main returned before eglSwapBuffers".to_string())
        }
    }
}

fn set_prop(object: &Object, name: &str, value: JsValue) -> Result<(), String> {
    Reflect::set(object.as_ref(), &JsValue::from_str(name), &value)
        .map(|_| ())
        .map_err(js_error_string)
}

fn js_error_string(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("JavaScript error: {value:?}"))
}

fn usize_value(value: usize) -> JsValue {
    JsValue::from_f64(value as f64)
}
