# Developing Fanta on Windows

Fanta ships for macOS, but the repository builds, runs and tests on Windows so
you can work on it there. The `Windows` workflow
([`.github/workflows/windows.yml`](../../.github/workflows/windows.yml)) keeps it
that way: on every pull request that touches code it checks the app and the
command-line tool and runs the tests of the canvas, the renderer and the design
engine crates on `windows-2025`.

## What is different from a Mac

- **The canvas renders on the CPU.** Skia's Metal and Graphite backends, and the
  GPU tile cache built on them, are macOS-only. Windows draws the canvas with the
  CPU raster renderer that also produces exports and agent screenshots, so very
  large documents pan more slowly than on a Mac.
- **No media playback.** Video playback and poster frames use AVFoundation, and
  the generation workspace's audio preview is macOS-only. On Windows, video
  layers and video fills show the poster frame saved with the project (a video
  placed on Windows has none), and audio previews do not play. Placing video
  and audio on the canvas works on both.
- **No release packaging.** `script/bundle-mac`, signing, notarization and the
  in-app updater are macOS-only. `script/bundle-windows.ps1` comes from upstream
  and has not been validated for Fanta.
- Tests that need Unix permission bits or symbolic links are `#[cfg(unix)]` and
  are skipped. Most code workspace tests and one session-restore test are
  ignored on Windows: they read real files and check the result before Windows
  has finished loading them.

Everything else, including the document engine, import, saving, the agent panel
and the live MCP server, is the same code.

## Set up

1. Windows 10 version 1803 or later, or Windows 11, on x64. The live MCP server
   uses AF_UNIX sockets, which need 1803.
2. **Visual Studio 2022 Build Tools** with the *Desktop development with C++*
   workload. It provides the MSVC linker and the Windows SDK that native
   dependencies need.
3. **Git for Windows**, with long paths on. The Rust target tree nests deeply:

   ```powershell
   git config --global core.longpaths true
   ```

   Also turn on Win32 long paths (*Enable Win32 long paths* in Group Policy, or
   `LongPathsEnabled` = 1 under `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem`),
   and clone into a short directory such as `C:\src\fanta-edit`.
4. **Rust** through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the
   version; the first `cargo` command installs it.

The build downloads upstream's prebuilt Skia for `x86_64-pc-windows-msvc`, so it
needs no LLVM or Python. If that download is blocked, install LLVM (for
`clang-cl`) and Python 3 and the build compiles Skia from source instead, which
also needs access to `skia.googlesource.com`. The app's build script downloads
Windows Terminal's ConPTY package from GitHub on the first build.

## Build, run and test

```powershell
cargo run -p fanta
```

The first build compiles the whole workspace and takes a while; later builds
are incremental.

```powershell
cargo test -p fanta-doc -p fanta-fnx -p fanta-format -p fanta-canvas -p fanta-tools -p fanta-present -p fanta-render -p fanta-text -p design_surface --lib
cargo test -p fig_viewer --lib
.\script\clippy.ps1 -p fig_viewer
```

A native crash inside Skia ends the whole test process and hides the tests
after it. `script\test-isolated.ps1` runs a crate's tests and, after a failure,
reruns each test in its own process to list every test that fails:

```powershell
.\script\test-isolated.ps1 -Package fanta-render
```

`script\clippy.ps1` uses the same flags as `script/clippy`. The other scripts in
`script/` are Bash; run them from Git Bash, or run the `cargo` commands they wrap
directly.

## Agents

Claude Code, Codex and other MCP clients connect to the running app through
`fanta.exe --mcp-stdio`, exactly as on a Mac; point the client at
`target\debug\fanta.exe`. See [`agent-quickstart.md`](agent-quickstart.md).
