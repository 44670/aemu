# Minecraft PE Local Probe

Local APK inspected:

```text
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

As of the latest local scan, this is also the only APK under:

```text
/mnt/hgfs/deb13/AndroidGames
```

Rechecked on 2026-05-12:

```sh
find /mnt/hgfs/deb13/AndroidGames -maxdepth 3 -type f \( -iname '*.apk' -o -iname '*.so' \) -print
find /mnt/hgfs/deb13 -maxdepth 5 -type f \( -iname '*minecraft*apk' -o -iname 'libminecraftpe.so' \) -print
```

Both searches still only found the APK listed above; no standalone
`libminecraftpe.so` and no older ARMv6 Minecraft PE APK were present.

Native libraries found:

```text
lib/armeabi-v7a/libfmod.so
lib/armeabi-v7a/libgnustl_shared.so
lib/armeabi-v7a/libminecraftpe.so
```

ZIP metadata for the local APK's native libraries:

```text
lib/armeabi-v7a/libfmod.so: deflated, 1,186,316 -> 595,071 bytes
lib/armeabi-v7a/libgnustl_shared.so: deflated, 5,548,472 -> 1,888,128 bytes
lib/armeabi-v7a/libminecraftpe.so: deflated, 23,554,092 -> 8,171,610 bytes
```

The APK/ZIP metadata exposes the compression method and sizes, but not a
reliable original deflate compression level.

`libminecraftpe.so` is not an ARMv6 target. The project CLI confirms ARMv7,
Thumb-2, VFPv3, and NEON:

```sh
cargo run -- probe-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

This APK is useful for HLE/API research, but not for validating the
ARMv6/`armeabi` interpreter path.

The APK run planner makes the loader-side blocker explicit:

```sh
cargo run -- plan-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

Result: no `lib/armeabi/*.so` group is present. All three native libraries are
under `lib/armeabi-v7a/` and require ARMv7/Thumb-2; `libfmod.so` and
`libminecraftpe.so` also require NEON.

Dynamic import inspection:

```sh
cargo run -- imports-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --limit 25
```

Observed dependency/import counts:

```text
libfmod.so: needs liblog.so, libstdc++.so, libm.so, libc.so, libdl.so;
  106 imports, 1,829 relocation entries
libgnustl_shared.so: needs libm.so, libc.so, libdl.so;
  117 imports, 2,719 relocation entries
libminecraftpe.so: needs libfmod.so, libgnustl_shared.so, liblog.so,
  libandroid.so, libGLESv1_CM.so, libEGL.so, libGLESv2.so, libOpenSLES.so,
  libz.so, libm.so, libdl.so, libc.so; 683 imports, 114,460 relocation entries
```

The parser also verifies representative Minecraft imports such as
`eglGetProcAddress` and `glCreateShader`, which confirms that the next runtime
work is relocation application plus binding imported symbols to Bionic,
Android, EGL/GLES, OpenSL, zlib, FMOD, and C++ runtime HLE surfaces.

Native linker probe:

```sh
cargo run -- link-apk /mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk --abi armeabi-v7a --limit 30
```

This is a loader/HLE-surface probe only because the APK is ARMv7. The probe
loads all three APK-local libraries into a 1:1 guest virtual address map:

```text
libfmod.so: load_bias 0x70000000, mapped 0x70000000+0x15c000
libgnustl_shared.so: load_bias 0x70300000, mapped 0x70300000+0xb6000
libminecraftpe.so: load_bias 0x70500000, mapped 0x70500000+0x1701000
```

After adding the initial system-library HLE import table, the same probe found
53,631 APK-local dynamic exports, reserved 462 guest HLE symbols, resolved 906
imports, and applied 119,008 relocation entries with zero unresolved imports.
The default ARMv6 command still fails correctly with:

```text
no native libraries found for ABI armeabi; available ABIs: armeabi-v7a
```

Graphics imports seen in the dynamic symbol table are GLES 2.0-style, not GLES
1.1 fixed-function-style. Examples include:

- shader/program APIs: `glCreateShader`, `glShaderSource`, `glCompileShader`,
  `glCreateProgram`, `glAttachShader`, `glLinkProgram`, `glUseProgram`
- attribute/uniform APIs: `glVertexAttribPointer`,
  `glEnableVertexAttribArray`, `glGetAttribLocation`, `glUniform*`,
  `glUniformMatrix*`
- buffer/framebuffer APIs: `glBindBuffer`, `glBufferData`,
  `glBufferSubData`, `glBindFramebuffer`, `glFramebufferTexture2D`,
  `glCheckFramebufferStatus`
- texture/draw/state APIs: `glTexImage2D`, `glTexSubImage2D`,
  `glTexParameteri`, `glDrawArrays`, `glDrawElements`, `glViewport`,
  `glScissor`, `glBlendFunc`, `glDepthFunc`
- `eglGetProcAddress`

No direct imports for common GLES 1.1 fixed-function names such as
`glMatrixMode`, `glVertexPointer`, `glTexCoordPointer`, or
`glEnableClientState` were found in this local library.

Browser translation implication: this symbol set maps to WebGL 1 better than
WebGL 2 as an initial baseline, because WebGL 1 is the browser form of the GLES
2.0 programming model. WebGL 2 should stay optional until a target APK or trace
requires GLES 3.x behavior or a WebGL 2-only extension.

HLE implication: Minecraft PE will need GLES 2.0 shader/VBO/FBO coverage, EGL
surface/context shims, APK asset/file access, libc/pthread/math/time shims,
network/socket stubs or implementation, and audio through FMOD/OpenSL/Android
bridging depending on the final older target APK.
