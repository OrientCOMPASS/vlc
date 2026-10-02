# libvlcrs — a lightweight Rust playback core with spherical video support

`libvlcrs` is a from-scratch Rust re-implementation of the parts of libvlc that a
player application actually needs on Android, built for a single ABI
(`arm64-v8a`) and shipped as one shared library (`libvlcrs.so`).

It exists because upstream libvlc's spherical support does not cover the format
matrix required by the VR playback feature:

| requirement | upstream libvlc | libvlcrs |
|---|---|---|
| 360° equirectangular, mono | yes (metadata driven only) | yes, **and forced on any source** |
| 180° equirectangular | no | yes |
| side-by-side / top-bottom stereo sources | no (left eye hard coded) | yes, **eye selectable at runtime** |
| forced planar (2D) rendering of a spherical source | partial | yes |
| `Auto` from container metadata | yes | yes (`st3d`/`sv3d`/`proj`/`prhd`/`equi`, Matroska `StereoMode`/`Projection`, Spherical V1 `uuid` XML) |
| 180° manual yaw converging at the coverage boundary | n/a | yes (never shows black) |
| in-place format/eye switching, position preserved | no | yes (uniform update on the render thread) |
| HUD read-outs (yaw/pitch/FOV/…) | no | yes |

The projection pipeline is a Rust port of the "decode → project" logic of
[xl_player](https://github.com/xl-player-developers/xl_player)
(`xl_mesh_factory.c`, `xl_mat4.c`, `xl_model_ball.c`, `xl_model_rect.c`,
`xl_head_tracker/*`), extended with the 180°/stereo/eye-selection cases above.

## Crates

| crate | target | contents |
|---|---|---|
| `crates/vlcrs-vr` | host + android | format matrix, equirectangular meshes, view control (drag/pinch/gyro/recenter), boundary convergence, Cardboard `OrientationEKF` port, HUD, GLSL sources, audio downmix |
| `crates/vlcrs-media` | host + android | ISO-BMFF + Matroska/WebM probing: tracks, duration, rotation and spherical metadata |
| `crates/vlcrs-lite` | android (`cdylib` → `libvlcrs.so`) | engine (demux/decode/audio/clock/seek), EGL + GLES renderer, JNI bridge, C ABI |

`vlcrs-vr` and `vlcrs-media` are platform free and carry the unit tests that can
run anywhere (`cargo test --workspace`); `vlcrs-lite` is compiled for
`aarch64-linux-android` only.

## Engine design

```
              ┌──────────────┐   packets    ┌───────────────────────────────┐
  media ─────▶│  demux thread │────────────▶│ video queue │ audio queue     │
 (path/fd/uri)│ AMediaExtractor│            └───────┬───────┬───────────────┘
              │ + container probe                   │       │
              └──────────────┘              ┌───────▼───┐ ┌─▼──────────────┐
                                            │  render   │ │  audio thread  │
                                            │  thread   │ │ AMediaCodec →  │
                                            │ AMediaCodec│ │ downmix →      │
                                            │ → Surface- │ │ AAudio (master │
                                            │   Texture  │ │    clock)      │
                                            │ → EGL/GLES │ └────────────────┘
                                            │  projection│
                                            └───────────┘        ┌──────────────┐
                                                                 │ sensor thread │
                                                                 │ ASensorManager│
                                                                 │ + EKF         │
                                                                 └──────────────┘
```

* **No bundled codecs.** Demuxing and decoding go through the platform
  (`AMediaExtractor`, `AMediaCodec`), audio output through AAudio. That is what
  keeps the engine lightweight: one `.so`, no FFmpeg, no contribs.
* **Zero-copy video.** The decoder renders into a `SurfaceTexture` owned by our
  own EGL context (created on the render thread through a small Kotlin bridge);
  the projection shader samples the resulting external OES texture.
* **Decoding and rendering share one thread.** `AMediaCodec` is not `Send`, and
  keeping it next to the code that releases its output buffers removes a whole
  class of lifetime bugs. Hardware decoding stays asynchronous: the thread only
  queues compressed packets and collects decoded buffers.
* **Audio is the master clock**; without an audio track the video path anchors
  the clock itself.
* **The application surface may come and go** (rotation, backgrounding) without
  touching the decoder: only the EGL *window* surface changes, the context, the
  OES texture and the decoder surface survive.

## VR model

* Sphere vertices: `(cos(lat)·sin(lon), sin(lat), −cos(lat)·cos(lon))`, so
  `lon = 0` (`u = 0.5`) faces the viewer at rest.
* Model matrix `Rz(roll) · Rx(−pitch) · Ry(yaw)` — the same order as the
  reference player, with pitch around the *yawed* local X axis, so the view
  centre is exactly `(lon = yaw, lat = pitch)` and vertical drag never
  degenerates into roll.
* Eye/layout selection is a `uUvRect` uniform applied to the texcoords *before*
  the `SurfaceTexture` matrix, i.e. in display space: switching eye or format is
  one uniform update — no re-upload, no pipeline restart, position preserved.
* 180° sources: manual yaw/pitch converge smoothly (`CONVERGE_KNEE = 0.72`) at
  `±(90 − fov/2)` so the picture can never be left behind; the hemisphere mesh
  is additionally tessellated over the full sphere with clamped `u`, so even
  extreme tilt shows the boundary column instead of black. Gyroscope mode
  relaxes the limit to the coverage edge, as required.
* FOV: 25°…120°, default 75°, pinch zoom maps to `fov_y` (1:1 drag sensitivity
  by default: a full screen width sweep rotates by the horizontal FOV).

## Building

Host tests, lints and formatting (any machine):

```sh
cd libvlcrs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --release -- -D warnings
cargo test --workspace --release
```

Android arm64-v8a (release only):

```sh
export NDK=/path/to/android-ndk          # r26 or newer
export TOOLCHAIN=$NDK/toolchains/llvm/prebuilt/linux-x86_64
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$TOOLCHAIN/bin/aarch64-linux-android26-clang
export AR_aarch64_linux_android=$TOOLCHAIN/bin/llvm-ar
rustup target add aarch64-linux-android
cargo build --release --target aarch64-linux-android -p vlcrs-lite
# → target/aarch64-linux-android/release/libvlcrs.so
```

CI does exactly this (see `.github/workflows/libvlcrs.yml`) and publishes the
artefact as a GitHub release.

## Tools

* `tools/spherical_inject.py` — writes Spherical Video V2 metadata (`st3d`,
  `sv3d/proj/prhd/equi`, plus the V1 `uuid` XML) into an MP4, or `StereoMode` +
  `Projection` into a Matroska/WebM file. CI uses it to tag generated test clips
  so `Auto` can be validated on device without committing large fixtures.
* `cargo run -p vlcrs-media --example probe -- <file>…` — prints what the prober
  sees and how every mode of the matrix resolves against it.

## Integration

* Kotlin/Java: the `libvlcrs-android` project in the
  [vlc-android](https://github.com/OrientCOMPASS/vlc-android) repository wraps
  the JNI surface in a small API (`RsMediaPlayer`, `RsMedia`, `VrView`) and ships
  a demo player.
* C/C++ or another FFI layer: `include/vlcrs.h` documents the handle based C ABI
  (`vlcrs_*`). Player instances are created from the Kotlin side (the surface and
  the event callback are Java objects) and can then be driven entirely from C.
