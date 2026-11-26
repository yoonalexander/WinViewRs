# WinViewRs

[![CI](https://github.com/yoonalexander/WinViewRs/actions/workflows/ci.yml/badge.svg)](https://github.com/yoonalexander/WinViewRs/actions/workflows/ci.yml)

## Overview

WinViewRs is an independent native Rust reimplementation of [WinSetView](https://github.com/LesFerch/WinSetView) by Les Ferch.

The eventual goal is a modern Windows Explorer configuration manager with safe previews, configuration profiles, and reversible changes.

The current implementation is a **read-only Explorer configuration inspector**. It reads relevant Windows registry domains and reports what it finds. Explorer settings cannot currently be modified through this application.

## Current status

- **Phase 0: Read-only inspection foundation** — implemented. Registry reads, value decoding, structured snapshots, CLI output, and resource-bounded traversal.
- **Phase 1: Read-only desktop GUI** — inspection overview and Windows metadata displayed. The native GUI launches and connects to the registry inspector.
- **Later phases**: Configuration planning, backups, restoration, and explicitly authorized application of changes.

## Features

Implemented capabilities:

- Windows version metadata detection using `RtlGetVersion` with a registry fallback.
- Separate HKCU and HKLM `FolderTypes` inspection.
- Default `TopViews` inspection.
- `Bags` and `BagMRU` inspection.
- `Streams` inspection.
- Advanced Explorer settings inspection.
- Selected CLSID overrides inspection.
- Explorer policy inspection.
- Raw registry value types and bytes.
- Decoded values where supported.
- Unknown and malformed value handling.
- Optional-key absence handling.
- Structured inspection errors.
- Human-readable, JSON, and TOML CLI output.
- Registry traversal and captured-payload limits.
- Feature-gated native GUI launcher (`cargo run -p winview-app --features gui -- gui`).

Inspection is limited to the configured registry domains listed above.

## Requirements

- Windows 10 or Windows 11 as intended application targets.
- Rust 1.100.0, pinned by `rust-toolchain.toml`.
- Standard Windows Rust/MSVC build prerequisites.
- No administrator privileges required for normal inspection.

Not all Windows versions or builds have been tested. Complete inspection status does not guarantee complete version metadata.

The `winview-core` crate is platform-neutral and tested on Linux, macOS, and Windows.

The complete application is Windows-specific.

## Getting started

```powershell
git clone https://github.com/yoonalexander/WinViewRs.git
cd WinViewRs

cargo build --workspace

cargo run -p winview-app -- --help

cargo run -p winview-app -- inspect

cargo run -p winview-app -- dump --format json

cargo run -p winview-app -- export

# Launch the native GUI (feature-gated)
cargo run -p winview-app --features gui -- gui
```

JSON is the preferred machine-readable inspection format.

TOML export is intended for debugging and may fail when a snapshot contains values that TOML cannot represent, such as QWORD values above `i64::MAX`. Neither format is currently a guaranteed restoration backup.

## CLI exit codes

| Code | Meaning |
|---|---|
| 0 | Complete inspection, or successful help/version command |
| 1 | Failed inspection or operational error |
| 2 | Invalid command, arguments, or unsupported output format |
| 3 | Partial inspection |

Partial and Failed snapshots are still emitted when output succeeds.

Serialization and I/O failures take precedence over inspection status.

Tracing output is written to stderr.

## Architecture

WinViewRs is organized as a three-crate Rust workspace:

- **winview-core**: Platform-neutral models, registry traits, shared inspection utilities, version capabilities, and test support.
- **winview-registry**: Windows registry access, native enumeration, decoding, inspection, and version detection.
- **winview-app**: CLI commands, output formatting, profile model, and feature-gated GUI.

```text
                  App Layer (winview-app)
          CLI: inspect / dump / export / gui
          GUI: egui/eframe (feature-gated)
                    |
                    v
          Core Layer (winview-core)
   InspectionSnapshot, RegistryValue, traits,
   shared error-counting utilities, version model
                    |
                    v
        Platform Layer (winview-registry)
   WindowsRegistryReader, native Win32 enumeration,
   RtlGetVersion detection, decoding, resource limits
```

`winview-core` builds on all platforms; `winview-registry` and `winview-app` are Windows-specific.

Dependency flow: `winview-app` depends on `winview-core` and `winview-registry`; `winview-registry` depends on `winview-core`. `winview-core` has no dependency on the Windows adapter.

## Safety

- Phase 0 and Phase 1 are read-only regarding Windows system configuration.
- No registry mutations.
- No Explorer restarts.
- No automatic elevation.
- No apply, restore, or reset functionality.

Registry snapshots may contain personal paths and other system-specific data. Users should inspect and redact snapshots before sharing them.

## Development

```powershell
cargo check --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Windows workspace CI runs `cargo check`, `cargo fmt --check`, `cargo clippy`, and `cargo test` on `windows-latest`. A separate Linux/macOS CI job builds and tests `winview-core` only.

There are 11 live-registry integration tests, all marked `#[ignore]`. They require direct Windows registry access and are not claimed to run automatically in CI.

## GUI (Phase 1 foundation)

The native GUI is an initial foundation. It can be launched with:

```powershell
cargo run -p winview-app --features gui -- gui
```

Current GUI state:

- Manual background inspection and refresh.
- Inspection status (Complete, Partial, Failed) and recorded error count displayed.
- Windows product name, build, UBR, detection method, and capture timestamp shown.
- Per-domain record and error summaries (7 domains).
- FolderTypes navigation with separate system-level (HKLM) and user-level (HKCU) records.
- Paginated folder-type browsing (25 records per page).
- Canonical names and GUIDs displayed for each folder type.
- Captured TopViews settings shown per view (LogicalViewMode, Mode, IconSize, GroupBy, etc.).
- Select a TopViews field to inspect its captured raw observation in a shared read-only detail panel, showing the value name, registry hive, captured key path, observation source, original numeric registry type ID, raw payload size, decoding status, a bounded hexadecimal raw payload preview, and a bounded decoded representation.
- Previews are bounded (64 bytes of hexadecimal, 160 characters of text, 8 multi-string entries) with explicit truncation indicators; the raw preview is **not** a complete raw payload dump.
- Fields without a corresponding captured raw observation are explicitly reported as having no raw observation, which is distinct from a failed read or a zero-length payload.
- The existing normalized field display is unchanged; selection only adds the raw detail panel below it.
- Explicit distinction between unavailable values and captured values (including empty strings).
- These records are captured registry snapshots, not necessarily the effective settings Explorer applies.
- Advanced settings browser with seven supported settings (HideFileExt, UseCompactMode, AutoCheckSelect, Hidden, LaunchTo, FullRowSelect, NoStrCmpLogical), numeric DWORD values, distinct read and decoding statuses, and read-only behavior.
- Select one of the seven supported Advanced settings to open its captured raw value details in the shared read-only detail panel; raw and decoded previews use the existing bounded renderer (64 hex bytes, 160 text characters, 8 multi-string entries).
- Missing and failed observations remain distinct; a setting with no captured observation is reported explicitly and never fabricated or treated as zero.
- Unexpected registry types can still be inspected in the detail panel without being treated as valid Advanced DWORD settings in the main table.
- Refresh clears the Advanced selection; switching GUI tabs preserves it.
- No configuration editing is supported.

### Registry domain browser

- Reusable read-only browser for Bags, Streams, CLSID, and Policies domains.
- Hierarchical key navigation with Roots and Up controls.
- Bounded key and value pagination (25 records per page).
- Registry type IDs, payload sizes, and read/decoding statuses.
- Select a captured value to inspect its details: value name (including `(Default)`), registry hive, captured key path, observation source, original numeric registry type ID, total raw payload size, decoding status, a bounded hexadecimal raw payload preview, and a bounded decoded representation.
- Previews are bounded (64 bytes of hexadecimal, 160 characters of text, 8 multi-string entries) with explicit truncation indicators; the raw preview is **not** a complete raw payload dump.
- Decoded data is shown only when decoding succeeded; malformed, unknown-type, and failed reads are reported honestly without fabricating bytes.
- Changing domains, entering keys, going Up, returning to Roots, changing the value page, or refreshing clears the current selection.
- All displayed information comes from captured read-only snapshots; no additional registry reads are performed.
- Detailed recorded inspection errors remain deferred.

## Known limitations

See `docs/PHASE_0_COMPLETION.md` for the complete list of known limitations and their classification.

## Roadmap

- Phase 0: Read-only foundation.
- Phase 1: Native read-only inspector GUI.
- Future phases: Typed configuration, change planning, backup, restore, and safe application.

No dates are promised.

## Attribution and license

WinSetView is by [Les Ferch](https://github.com/LesFerch/WinSetView) and is licensed under the MIT License.

WinViewRs is an independent Rust reimplementation of WinSetView, not an official WinSetView release, and is not affiliated with or endorsed by the original author.

WinViewRs is released under the MIT License. See `LICENSE`. The original copyright and license notices are preserved, and Les Ferch is clearly attributed for reused or adapted material.

