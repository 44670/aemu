# No-Fake-HLE Contract and Audit

## Scope

AEMU is an Android HLE runtime, so Android, bionic, EGL, GLES, JNI, filesystem,
input, and other missing platform services remain legitimate HLE boundaries.
HLE is not allowed to replace code supplied by the APK or to invent successful
game behavior. This distinction is structural, not based on whether a frame
looks plausible.

That whole-APK contract is **not yet satisfied**. Current evidence closes only
the native symbol-dispatch gate: APK-defined game and engine exports are not
replaced by HLE, except for the explicit compiler integer-helper overrides.
Two independent gates remain open:

1. Native lifecycle integrity. The launcher reaches the APK's real
   `ANativeActivity_onCreate` and pthread entry, but then overwrites the
   guest-created `android_app` and injects lifecycle poll sources instead of
   driving the callbacks and command pipe installed by the APK.
2. APK Java execution. AEMU does not execute DEX bytecode and instead contains
   bounded Rust semantic lifts for selected APK-defined Java methods.

Strict symbol scans, DEX declaration checks, plausible return values, and
rendered screenshots make these shortcuts auditable; they do not make the
omitted control flow execute. The overall no-fake gate therefore remains open.

Production dispatch has four outcomes:

1. `Implemented`: bounded semantics validate arguments and update every modeled
   guest-visible return value, output, state, side effect, and error.
2. `RuntimeImplemented`: synchronization whose completion belongs to the guest
   scheduler rather than the import dispatcher.
3. `Unimplemented`: an exact-call trap, with no synthetic return.
4. `Abort`: an exact-call fatal condition such as `abort`, stack-check failure,
   assertion failure, or compiler-runtime division by zero.

There is no generic return-zero, return-one, or void-success behavior. A symbol
marked implemented but missing a dispatcher raises `MissingImplementation`.
Unknown JNI table entries and unknown registered JNI methods also stop at the
exact call.

## Native-Code Invariants

- `HleSymbolKind` contains only system/platform families. It has no target or
  game kind.
- The HLE classifier contains no MCPE mangled methods. Tests reject
  representative rendering, resource, input, geometry, telemetry, and image
  loader symbols.
- HLE entries represent system imports, plus the explicit AEABI/libgcc
  integer-helper override list in `is_defined_hle_override_symbol`. APK-defined
  game and engine symbols remain native; the overridden compiler helpers have
  arithmetic and divide-by-zero tests.
- The local MCPE link test mechanically scans every symbol defined by
  `libminecraftpe.so`. Any overlap with HLE must be one of those compiler
  helpers, and every other relocation for a game-defined name must have a
  native source.
- There is no environment-controlled game dispatcher, game resource bridge,
  target method fixture, or selective guest-thread allowlist in production.

## Native-Activity Required Invariants

`ANativeActivity_onCreate` must install `ANativeActivity.instance`. Failure to
do so is fatal; launch does not allocate a substitute `android_app`.
`pthread_create` queues every non-null start routine. The launcher locates the
queued thread whose argument is that installed app, promotes its real CPU and
pthread state, and continues it to `eglSwapBuffers`. Both desktop and
WebAssembly use this path. Neither launcher resolves or invokes `android_main`
directly.

Guest worker threads execute their supplied routines. Mutexes, condition
variables, `pthread_once`, cleanup handlers, per-thread errno/TLS state, and
bounded pthread-key destructor passes are scheduler-visible. Unsupported join,
detach, semaphore, or other thread APIs trap rather than claim success.

Those facts are necessary but not sufficient. The current launch path still
violates the lifecycle gate in four ways:

- `src/main.rs` and `src/wasm_api.rs` hard-code `libminecraftpe.so`, call every
  linked `JNI_OnLoad` from the host, and call
  `Java_com_mojang_minecraftpe_MainActivity_nativeRegisterThis` directly. This
  bypasses APK class initialization, constructor execution, Java virtual-call
  dispatch, and native-library load ordering. It also calls
  `nativeRegisterThis` before `ANativeActivity_onCreate`, whereas this APK's
  `MainActivity.onCreate` calls `NativeActivity.onCreate` first and
  `nativeRegisterThis` later.
- After the real `ANativeActivity_onCreate` returns, `populate_android_app`
  overwrites the guest-created app's configuration, looper, pending/current
  input queue and window, poll source, and lifecycle fields. Ownership of this
  structure belongs to the APK's native-app-glue code.
- `queue_android_lifecycle_events` writes a host-authored ARM callback into
  guest memory and queues synthetic START, RESUME, INIT_WINDOW, and
  GAINED_FOCUS sources. This bypasses the callbacks installed in
  `ANativeActivity.callbacks`, the native-app-glue command pipe,
  `android_app_read_cmd`, `android_app_pre_exec_cmd`, the APK's static
  `process_cmd`, and `android_app_post_exec_cmd`.
- Activity/library/package selection is inferred from DEX hierarchy with MCPE
  fallback strings instead of being selected from parsed manifest metadata.

The unit test `native_activity_queues_lifecycle_alooper_sources` currently
asserts the synthetic queue itself. Its success protects the shortcut; it is
not evidence for the required framework callback and command-pipe path.
The platform HLE already models `pipe`, byte transport, `ALooper_addFd`, and
poll readiness, so the synthetic source is not forced by an absent transport
primitive. The missing work is framework callback invocation plus scheduler
handling while the app-glue side performs its normal synchronous handshakes.

The tested APK's actual `MainActivity.onCreate` also creates platform and input
objects, initializes FMOD, starts the platform view, installs a headset
receiver, reports headset state to native code, sets `mInstance`, and records
`_fromOnCreate`. The direct native call executes none of those DEX operations.
The promoted pthread is real, but the host-synthesized control path used to
reach it is not a faithful Android lifecycle.

## Platform-State Invariants

- Offline TCP connect and external socket writes fail with `ENETUNREACH`;
  unavailable receives report `EAGAIN`. A bounded in-process UDP queue handles
  only local endpoints, including a socket's own bind-address self-test.
  `SIOCGIFCONF` exposes the modeled loopback interface with the Android 4.4 ARM
  `ifconf`/`ifreq` layout. Poll/select/epoll derive readiness from queued data
  and descriptor state instead of clearing inputs and reporting success.
- Bionic `FILE` uses the Android 4.4 ARM layout. `__sF` initializes real
  stdin/stdout/stderr flags and descriptors; invalid or closed streams no
  longer make `fclose`, `fputs`, `fputc`, `fwrite`, `fprintf`, or `fflush`
  succeed.
- EGL tracks initialization, ownership, current context/surfaces, object
  lifetimes, and sticky errors. GLES tracks object lifetimes, shader compile and
  program link state, framebuffer completeness, exact upload/attribute payload,
  and sticky first-error semantics.
- GLES draw replay consumes the client-attribute payloads captured for the
  current linked program. Stale enabled client arrays at inactive locations do
  not suppress a draw; the interaction gate reports zero skipped client-array
  draws. This fixes the intermittent black rectangles and missing Play-screen
  controls seen in earlier captures.
- Motion and key input have bounded state and handle lifetime. Accessors return
  the active event's action, code, metadata, repeat count, pointer IDs, axes,
  and coordinates. Unmodeled input operations trap. Assets and virtual files
  return actual payload/state or explicit absence/failure.
- JNI class, method, field, staticness, direct/virtual category, native flag,
  superclass, and interface declarations come from every `classes*.dex` in
  the APK. Lookup follows the Dalvik 4.4 JNI search shapes; call targets are
  checked for assignability. Unknown classes/members set the corresponding
  exception instead of allocating a permissive handle.
- JNI direct, `V`, and `A` call layouts remain distinct. Constructor calls
  require a real non-static `<init>` declaration; the observed `(J)V` payload
  is decoded with AAPCS32 alignment and retained in the object proxy. Strings
  and arrays have typed handles and checked bounds. JavaVM attachment and
  pending exceptions are per guest thread. Unknown entries and unimplemented
  methods stop at the exact call.

One target-specific filesystem shortcut remains outside those invariants:
`virtual_dir_exists` unconditionally claims that the four MCPE parent paths
from `/sdcard` through `/sdcard/games/com.mojang/minecraftpe` exist. It does not
create a world or any file contents, but it changes `stat`, `access`, `open`,
and `mkdir` outcomes without an install-state or guest-created directory. These
roots must be initialized by a package/storage model or guest calls, not by
target path literals.

## APK Java Semantic Lifts

The startup path currently lifts these APK-defined Java families rather than
executing their DEX instructions. More generally, JNI `NewObject`,
`NewObjectV`, and `NewObjectA` validate that a constructor is declared but then
allocate a Rust-managed proxy; they do not execute `<init>`. JNI method calls
route through Rust signature matches rather than a DEX interpreter.

- `MainActivity`: screen/platform queries, preferences-backed device identity,
  locale/storage/network state, image/file loading, localization updates, and
  the exact `vibrate(I)V` wrapper into Android's vibrator service.
- `HardwareInformation`: Android release, model, ABI, CPU information, and core
  count derived from AEMU's declared Android 4.4 ARM platform.
- `StoreFactory`/`GooglePlayStore`: proxy allocation and an unavailable Google
  Billing outcome. Rust directly resolves and invokes the APK's native
  `NativeStoreListener_onStoreInitialized` export. That avoids a false success,
  but it still bypasses the APK's factory method, constructor, billing setup,
  listener dispatch, and callback timing.
- Xbox `Interop`: `ReadConfigFile` returns the APK's actual
  `res/raw/xboxservices.config`; the absent system proxy is represented as an
  empty Android proxy setting.

These methods are declaration-checked against the tested APK and unsupported
signatures fail closed. They do not create worlds, choose menu screens, mark
resources complete, or call MCPE rendering/game functions. They nevertheless
remain supplied APK code, so screenshots and native-symbol audits cannot close
the whole-APK contract by themselves.

The activity lifts are also keyed to whichever first DEX class is found to be
assignable to `android.app.NativeActivity`, not to a manifest-selected
component. A different APK with similarly named methods could therefore
receive MCPE-derived behavior even though every individual declaration check
passes.

## Current MCPE Evidence

The post-audit link has three native objects, 53,631 native exports, 509 HLE
reserved symbols, 906 relocation-bound imports, and zero unresolved imports.
The 644 HLE relocation bindings are all system/compiler boundaries: 518
implemented, 17 scheduler-runtime, 101 exact unsupported traps, and 8 abort
entries. The exhaustive local link test scans every symbol defined by
`libminecraftpe.so` and rejects any HLE overlap outside the explicit compiler
integer-helper override list.

The current release interpreter run uses the real native-app-glue pthread and
no Dynarmic, game hook, target resource bridge, skipped pthread, or fallback
app allocation. It still uses the direct JNI/lifecycle injection described
above, so this is rendering and native-execution evidence, not a passing
no-fake run:

```text
artifact: tmp/mcpe-strict-jni-20260716-final5
backend: aemu interpreter
first eglSwapBuffers step: 310629057
frames/swaps: 300/300
indexed draws: 5979
maximum readback RGB: 406452 of 409920 pixels
replay GL errors: 0
process: exit 0 in 31.401 seconds
captured per-draw PNGs: 2390
skipped client-attribute draws: 0
```

The interaction gate in `tmp/mcpe-ui-strict-jni-20260716-final3` captures the
rendered Xbox dialog, the title menu after selecting `Not Now`, and the Play
screen after selecting `Play`. The last frame shows the native
Worlds/Realms/Friends UI with `Worlds 0` and `Create New World`; the landscape
behind the panel is the animated title panorama, not an entered or synthesized
playable world. The three screenshot SHA-256 values are, in order,
`37ee2931a5bc0f4767698f052cf9aeaa1e727fdbea1563e6f8f5f0fe0bc80c4d`,
`5468def5a38d9404f48f0edbe2d211956a4a6a3cde1ab226bee2b352f0e2cd76`, and
`d5a799deebdac5f3644b25938ec0507d69510d867dddd1b4225faef50aad59e0`.
This establishes that AEMU did not manufacture a saved world. It does not
establish Java execution or lifecycle integrity.

The repeatable `tools/mcpe_ui_smoke.py --preset playable-flat-world` gate now
continues through `Create New World`, selects the game's native Advanced/Flat
controls, and rejects menu/loading frames with three entered-world pixel gates
at 6, 21, and 36 seconds. It then drives the real input queue to hold forward,
rotate and pitch the camera, place a stone block, and long-press to break it.
Machine checks distinguish motion from passive frame changes and require the
gray placed block followed by the exposed dirt hole.

The release interpreter artifact
`tmp/mcpe-playable-flat-world-gate-20260716` passes all five interaction checks
with movement/camera/aim changed-channel ratios of 0.421/0.390/0.758, a placed
stone ratio of 0.647, and a post-break stone/hole ratio of 0.107/0.490. Its
final sample reports 1,924 frames, 105,475 indexed draws, 409,824 nonzero-RGB
pixels, zero skipped client-attribute draws, and zero replay GL errors. This
closes the concrete new-world entry and bounded playability smoke requested
here; it still does not close the independent native-lifecycle or APK-Java
execution gates above.

The tested release binary SHA-256 is
`324fa3bdf19648d63028df0bd53fb28a95ebb437ed61fd72c7364a48ae57eab9`; the
APK SHA-256 is
`ee6380c3d29b39744488acd7b986290d43037f4b210595889cec5c5a0ea04cdb`.
Any later loader, launch, threading, HLE, EGL, GLES, or JNI change must repeat
the long run and inspect rendered interaction frames rather than accepting
counters alone.

## Semantic References Consulted

- `../aemu-refs/aosp-bionic-4.4.4_r2/libc/kernel/common/linux/if.h` and
  `sockios.h` define the ARM-visible `ifconf`/`ifreq` fields, `IFNAMSIZ`, and
  `SIOCGIFCONF` request used by the socket model.
- `../aemu-refs/aosp-dalvik-4.4.4_r2/vm/Jni.cpp` and `vm/oo/Object.cpp` define
  the distinct `NewObject`/`V`/`A` layouts and the method/field hierarchy
  searches used by the DEX declaration index. `CheckJni.cpp` was used to audit
  target and ID validation.
- The tested APK's `classes*.dex` disassembly was checked for the exact
  `MainActivity` lifecycle, `HardwareInformation`, store, and Xbox Interop
  bodies. `MainActivity.onCreate` confirms that the current direct
  `nativeRegisterThis` call has the wrong context and ordering. This is
  evidence for auditing the listed lifts, not evidence that AEMU executes the
  DEX bodies.
- The tested APK's `MainActivity.vibrate(I)V` body loads the literal
  `"vibrator"`, calls `Context.getSystemService`, casts the result to
  `android.os.Vibrator`, sign-extends the duration to a Java `long`, and calls
  `Vibrator.vibrate(J)V`. The bounded lift records that platform request; it
  does not invent a game result. Android's 4.4 framework defines the
  `"vibrator"` service and the corresponding `vibrate(long)` operation.
- AOSP [`NativeActivity.java`](https://android.googlesource.com/platform/frameworks/base/+/1e4b9f3936d6f357e89360293e05a0e16d5fa440/core/java/android/app/NativeActivity.java),
  [`android_app_NativeActivity.cpp`](https://android.googlesource.com/platform/frameworks/base/+/65c83b906d01c3c1493d0547757dbb16d4c3722a/core/jni/android_app_NativeActivity.cpp),
  and [`android_native_app_glue.c`](https://android.googlesource.com/platform/ndk.git/+/refs/heads/main/sources/android/native_app_glue/android_native_app_glue.c)
  were checked for the framework-to-native callback chain and the app-glue
  command-pipe/read/pre/process/post sequence. The current synthetic
  poll-source path does not follow that sequence.
- RakNet's `RNS2_Berkley::BindShared` source establishes that startup sends a
  datagram to the socket's own bound address as a real send/receive self-test.
  AEMU models that local packet instead of returning generic send success.

## Acceptance Gate

1. Run formatting, the complete test suite, default-feature checks, SDL2
   checks, and the wasm build.
2. Link the local APK and record object/export/HLE/import counts. There must be
   no unresolved import and no APK-defined native HLE binding outside the
   explicit compiler-helper list. This closes only the symbol-dispatch gate.
3. Select the launch activity, native library, and package from parsed manifest
   data. Do not contain target package, class, JNI symbol, or library fallbacks
   in the production launcher or platform filesystem defaults.
4. Execute APK class initialization, the activity constructor, and
   `MainActivity.onCreate` in DEX order. Native library loads and `JNI_OnLoad`
   must occur because that code requested them, not because the launcher scans
   all linked objects.
5. Deliver start/resume/window/input/focus transitions through the framework
   `ANativeActivityCallbacks` installed by guest code. Require a trace through
   the real app-glue pipe and `read_cmd`/`pre_exec`/`process_cmd`/`post_exec`.
   The host must not overwrite a guest-created `android_app`, inject a process
   thunk, or queue app-glue command sources directly.
6. Run an interpreter SDL2 smoke for at least 300 frames with no Dynarmic,
   target hook, game bridge, lifecycle injection, skipped callback, or fallback
   app path. Require 300 swaps, at least 2,000 indexed draws, nonblank readback,
   and zero replay GL errors.
7. Run the `playable-flat-world` UI preset and require all three entered-world
   pixel gates plus movement, camera, block-placement, and block-breaking
   checks after the real create/loading sequence. Visual evidence supplements
   the structural audit; it never replaces it.
8. Report the symbol-dispatch, native-lifecycle, and whole-APK/DEX gates
   separately. Under the current absolute contract, the overall gate cannot
   pass while either lifecycle control flow is synthesized or any APK-defined
   Java method is supplied by a Rust semantic lift.
