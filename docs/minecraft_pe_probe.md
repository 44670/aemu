# Minecraft PE Local Probe

Local APK inspected:

```text
/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk
```

As of the latest local scan, this is also the only APK under:

```text
/mnt/hgfs/deb13/AndroidGames
```

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
