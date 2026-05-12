# Research Plan

This is the working plan for the Android 4.x HLE emulator target: old Android
games, ARMv6/`armeabi` native code, SDL2 for native debugging, and
wasm/WebGL for browser execution.

## Plan First

1. Keep the project as one Rust crate with narrow modules instead of a
   multi-crate workspace.
2. Treat the APK as an input container: inspect ZIP metadata, parse the
   manifest/resources/assets as needed, extract or stream files, and load
   `lib/armeabi/*.so` through our own ELF/linker path.
3. Run native game code with a custom ARMv6 ARM/Thumb interpreter. Use QEMU,
   Dynarmic, and Unicorn as references and test oracles, not as embedded
   runtime cores.
4. HLE Android and Bionic APIs on demand: libc, pthreads, time, file paths,
   APK assets, JNI/native-activity entrypoints, EGL, GLES, input, audio, and
   save data.
5. Use GLES as the guest-facing graphics API. Translate GLES 2.0 to WebGL 1
   where possible, emulate GLES 1.1 fixed-function over shaders, and add WebGL
   2 only for a concrete target-game requirement.
6. Use SDL2 as the native debug shell. For wasm, prefer the path that keeps
   SDL2 input/audio/windowing and WebGL context ownership straightforward.
7. Make Minecraft PE the first target, but require an older APK containing
   `lib/armeabi/libminecraftpe.so` before declaring ARMv6 validation complete.

## Research Decisions

### Dalvik and `libdvm`

Running the Dalvik interpreter directly is not enough to run an APK. Dalvik
executes DEX bytecode; Android app execution also depends on package install
state, resources, class loading, JNI, native libraries, lifecycle objects,
window/input/audio services, Binder-facing framework behavior, and Bionic
process state.

`libdvm` from Android 4.4 is still valuable as a source reference. Reusing it
as this project's runtime is the wrong default because it is C/C++, tied to
AOSP build assumptions and Android system libraries, includes JIT/interpreter
configuration paths, and does not solve ARM native `.so` execution in the
browser. The runtime should HLE the specific Java/JNI surface demanded by the
game instead.

### APK Install and Extraction

An APK is a ZIP-format package. Android install does not simply run the APK in
place; it records package metadata, validates/signs, exposes resources/assets,
and makes native libraries available to the app. On old Android releases,
compressed native libraries are normally extracted to an app library directory.
Modern Android can avoid extraction for page-aligned uncompressed libraries
when `android:extractNativeLibs` is false, but that optimization should not be
assumed for Android 4.x-era targets.

ZIP metadata stores the compression method and compressed/uncompressed sizes.
It does not reliably preserve the original deflate compression level. The local
Minecraft PE 0.15.0.1 APK stores its native libraries as deflated entries, with
`libminecraftpe.so` compressed from 23,554,092 bytes to 8,171,610 bytes.

### ARMv6 Interpreter

Use the ARM architecture manuals as semantic authority and use Dynarmic/QEMU to
cross-check decoder organization and behavior. The interpreter should implement
ARMv5TE plus ARMv6 user-mode ARM and Thumb-1, CP15 TLS reads/writes needed by
Bionic, and VFPv2 where target libraries require it. Privileged instructions
should trap or become explicit user-mode stubs instead of silently pretending
to emulate kernel behavior.

### Graphics and WebGL

WebGL 1 is the right first browser backend for Minecraft PE-style GLES 2.0
imports because WebGL 1 follows the OpenGL ES 2.0 shader pipeline model. WebGL
2 corresponds to OpenGL ES 3.0-style functionality and should be optional until
a target APK or trace needs it.

The local Minecraft PE 0.15.0.1 library imports GLES 2.0-style APIs such as
shader/program, attribute/uniform, VBO/FBO, texture, draw, viewport, scissor,
blend, and depth functions. It does not directly import common GLES 1.1
fixed-function names. The local APK is ARMv7/Thumb-2/VFPv3/NEON, so it is good
for HLE symbol research but not for ARMv6 validation.

ANGLE can help native desktop testing when a consistent GLES implementation is
useful, but it does not replace the browser WebGL backend.

## Milestones

1. Probe APKs and native libraries.
   Current status: implemented for native library ELF attributes, ZIP
   compression metadata, and pure-Rust stored/deflated ZIP entry extraction.
2. Finish the ARMv6 interpreter audit with focused QEMU oracle tests.
   Current status: broad ARMv6/VFPv2 coverage exists, but the audit is not
   complete.
3. Add an ELF loader/linker for `armeabi` `.so` files and imported-symbol
   dispatch into HLE shims.
   Current status: initial APK run planning selects `lib/armeabi` as the only
   ARMv6 interpreter ABI and reports concrete blockers for incompatible native
   libraries before loading. Initial ELF `PT_LOAD` segment planning and
   `VecMemory` materialization exists; dynamic sections, relocations, imported
   symbols, and dependency ordering are still pending.
4. Build Bionic/libc/pthread/time/file/memory shims only as demanded by target
   imports.
5. Implement EGL facade and GLES 2.0 command translation to SDL2 GL/WebGL.
   Current status: initial host abstraction exists, with a feature-gated SDL2
   GLES2 debug shell and WebGL 1/WebGL 2 context-selection scaffolding.
6. Add GLES 1.1 fixed-function emulation over shaders for older games that
   require it.
7. Add input/audio/storage HLE and enough Android lifecycle/JNI glue to reach
   the first frame.
8. Build the wasm/WebGL target and keep desktop SDL2 as the debugger target.
   Current status: the crate checks for `wasm32-unknown-unknown` with the
   `webgl` feature; a full browser harness is still pending.

## Sources Checked

- Android app fundamentals and APK packaging:
  https://developer.android.com/guide/components/fundamentals
- Android `android:extractNativeLibs` manifest behavior:
  https://developer.android.com/guide/topics/manifest/application-element#extractNativeLibs
- Android NDK ABI documentation:
  https://developer.android.com/ndk/guides/abis
- Android runtime documentation:
  https://source.android.com/docs/core/runtime
- WebGL 1.0 specification:
  https://registry.khronos.org/webgl/specs/latest/1.0/
- WebGL 2.0 specification:
  https://registry.khronos.org/webgl/specs/latest/2.0/
- Emscripten OpenGL/WebGL support notes:
  https://emscripten.org/docs/porting/multimedia_and_graphics/OpenGL-support.html
- Local AOSP Dalvik 4.4.4 reference:
  `../aemu-refs/aosp-dalvik-4.4.4_r2`
- Local Dynarmic reference:
  `../aemu-refs/dynarmic`
