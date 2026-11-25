# Phase 0 Completion Report

WinViewRs — Milestone 0.5.0

## 1. Baseline commit

- SHA: `bf6da1c45fcc042376304615727acf8ce36e37fc`
- Subject: "Milestone 0.4.9e: Consolidate shared inspection utilities"
- Working tree: clean

## 2. Actual workspace architecture

WinViewRs is a virtual Cargo workspace with three crates:

| Crate | Responsibility | Platform |
|---|---|---|
| `winview-core` | Platform-neutral models, registry traits, shared inspection utilities, version capabilities, test support | Cross-platform (Linux, macOS, Windows) |
| `winview-registry` | Windows registry access, native enumeration, decoding, inspection, version detection | Windows-only |
| `winview-app` | CLI commands, output formatting, profile model, feature-gated GUI skeleton | Windows-only |

Layering:

```text
App Layer (winview-app)
  CLI: inspect / dump --format json / export
  GUI: egui/eframe skeleton behind the `gui` feature flag
       |
       v
Core Layer (winview-core)
  InspectionSnapshot, RegistryValue, RawRegistryValue,
  RegistryReader/RegistryKey traits, shared error-counting utilities,
  version model (VersionInfo, DetectionMethod, FeatureCapabilities)
       |
       v
Platform Layer (winview-registry)
  WindowsRegistryReader, borrowed native Win32 handles (RegEnumValueW,
  RegEnumKeyExW), windows-registry crate, RtlGetVersion detection,
  decoding, resource-bounded traversal
```

`winview-core` builds on all platforms. `winview-registry` and `winview-app` are Windows-specific. The `gui` feature is off by default.

Dependency flow: `winview-app` depends on `winview-core` and `winview-registry`; `winview-registry` depends on `winview-core`. `winview-core` has no dependency on the Windows adapter.

The upstream reference repository at `../WinSetView-reference` is present and strictly read-only; it was not modified during this milestone.

## 3. Implemented inspection domains

Each domain is inspected read-only. Absent optional keys are handled gracefully without failing the run.

| Domain | Hive | Key path | Presence policy |
|---|---|---|---|
| FolderTypes | HKCU | `Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes` | Optional |
| FolderTypes | HKLM | `Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes` | Required (missing produces error) |
| TopViews | Per folder-type GUID | `...\FolderTypes\{GUID}\TopViews\{00000000-...-0000}` | Optional per folder type |
| Bags | HKCU | `...\Local Settings\...\Shell\Bags` | Optional |
| BagMRU | HKCU | `...\Local Settings\...\Shell\BagMRU` | Optional |
| AllFolders\Shell | HKCU | `...\Local Settings\...\Shell\Bags\AllFolders\Shell` | Optional |
| AllFolders\ComDlg | HKCU | `...\Local Settings\...\Shell\Bags\AllFolders\ComDlg` | Optional |
| AllFolders\ComDlgLegacy | HKCU | `...\Local Settings\...\Shell\Bags\AllFolders\ComDlgLegacy` | Optional |
| Streams | HKCU | `...\Explorer\Streams` | Optional |
| Streams\Defaults | HKCU | `...\Explorer\Streams\Defaults` | Optional |
| Streams\Desktop | HKCU | `...\Explorer\Streams\Desktop` | Optional |
| Advanced | HKCU | `...\Explorer\Advanced` | Optional |
| CLSID overrides | HKCU | 11 configured CLSID paths | Optional |
| Explorer policies | HKCU/HKLM | `...\Policies\Explorer`, `...\Policies\ComDlg32\PlacesBar` | Optional |

Advanced settings use a fixed list of supported value names: `HideFileExt`, `UseCompactMode`, `AutoCheckSelect`, `Hidden`, `LaunchTo`, `FullRowSelect`, `NoStrCmpLogical`.

## 4. Registry value decoding behavior

Raw bytes are always preserved regardless of decode outcome.

Raw bytes are preserved for captured `RawRegistryValue` observations, subject to capture limits. `CanonicalName` is a documented exception (retained as a decoded string, not a complete raw restoration record).

Recognized types are decoded into `RegistryValue`:

- `REG_SZ` → `String`
- `REG_EXPAND_SZ` → `ExpandString`
- `REG_DWORD` → `Dword(u32)` (malformed if length != 4)
- `REG_QWORD` → `Qword(u64)` (malformed if length != 8)
- `REG_BINARY` → `Binary(Vec<u8>)`
- `REG_MULTI_SZ` → `MultiString(Vec<String>)` (empty buffer → `[]`; missing double-null terminator → malformed)

Decoding status:

- `Decoded` — value successfully decoded.
- `Malformed { reason }` — recognized type but data is wrong length or invalid UTF-16/UTF-16LE; raw bytes preserved, `decoded = None`.
- `UnknownType` — unrecognized Windows type ID (including `REG_NONE` as `Other(0)`); raw bytes preserved, `decoded = None`.

Invalid UTF-16 registry names are reported as errors rather than converted lossily. `CanonicalName` is retained as a decoded string, not a complete raw restoration record.

## 5. Inspection status classification

`InspectionStatus` has three states:

| State | Meaning |
|---|---|
| `Complete` | No read errors and no limit breaches. |
| `Partial { reason }` | Limit breached (depth, key count, value count, value size, snapshot size) or read errors encountered while some data was captured. |
| `Failed { error }` | Read errors encountered with no successful observations captured. |

Status precedence in `finalize_inspection_status`:

1. Depth limit → `Partial`
2. Key count limit → `Partial`
3. Value count limit → `Partial`
4. Value size limit → `Partial`
5. Snapshot size limit → `Partial`
6. Total errors == 0 → `Complete`
7. Has successful observations → `Partial` (read errors present)
8. Otherwise → `Failed`

Partial and Failed snapshots are still emitted when output succeeds. Serialization and I/O failures take precedence over inspection status.

## 6. Optional-key presence policies

`KeyPresence` has two variants:

- `Optional` — a missing key produces no error and no snapshot entry; traversal stops silently at that root.
- `Required` — a missing key is an error condition (used for recursive child traversal under an opened root).

FolderTypes HKCU missing is tolerated and returns early. HKLM FolderTypes open failure is recorded as a read error. TopViews key absence is recorded only for non-`KeyNotFound` errors. Advanced key absence returns early. All Bags/Streams/CLSID/policies roots are optional.

## 7. CLI output and exit-code contracts

Commands:

- `inspect` — human-readable summary to stdout.
- `dump --format json` — full snapshot as pretty JSON to stdout.
- `export` — full snapshot as TOML to stdout (lossy; intended for debugging).

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Complete inspection, or successful help/version command |
| 1 | Failed inspection or operational error |
| 2 | Invalid command, arguments, or unsupported output format |
| 3 | Partial inspection |

`Complete` → 0, `Partial` → 3, `Failed` → 1. Serialization and I/O failures take precedence over inspection status. Registry snapshots are emitted to stdout; tracing and operational error diagnostics may be written to stderr.

## 8. Resource limits

| Limit | Value |
|---|---|
| Recursive depth threshold | 32 |
| Key-open attempts per recursive domain | 50,000 |
| Shared FolderTypes key-open attempts across HKCU/HKLM | 50,000 |
| Values captured per generic recursive key | 1,000 |
| Maximum individual captured registry value | 1 MiB |
| Shared captured source-payload budget | 50 MiB |

Clarifications:

- Recursive key openings are permitted at depths 0 through 31. An attempted descent to depth 32 produces a depth-limit record; the depth-32 path is never opened.
- Failed and missing key openings consume key-opening budget.
- The 1,000-value cap applies to generic recursive inspection, not every registry API operation.
- The 50 MiB limit concerns accepted source payload bytes, not total process memory.
- HKCU and HKLM FolderTypes share a single traversal budget.
- Advanced uses a fixed list of supported settings.
- Resource truncation produces `Partial` status.
- Depth-limit placeholders are not counted as actual key-open attempts and may increase snapshot size.
- Individual registry payload reads can allocate memory before individual-value and snapshot-budget checks.

## 9. Read-only safety audit findings

A review of production Rust code across all three crates confirmed the following:

1. `RegistryReader` and `RegistryKey` expose only read operations (`open_key`, `enum_keys`/`enum_subkeys`, `get_value`, `get_value_type`, `inspect_all`). No write, set, delete, create, restore, load, save, replace, or copy methods exist on either trait.
2. No `RegistryWriter` implementation exists anywhere in the workspace.
3. No production code creates, modifies, or deletes registry keys or values. Searches for `RegSetValue`, `RegDelete`, `RegCreateKey`, `RegRenameKey`, `RegRestoreKey`, `RegLoadKey`, `RegSaveKey`, `RegReplaceKey`, `RegCopyTree`, `RegistryWriter`, `Set-ItemProperty`, `Remove-ItemProperty`, `reg add`, `reg delete` found no production matches. These strings appear only in documentation/planning material.
4. No code terminates or restarts Windows Explorer. No `taskkill`, `Stop-Process`, or Explorer-refresh calls exist.
5. No elevation or administrator prompts exist. No `runas` or elevation logic exists.
6. Native Win32 enumeration uses borrowed handles (`HKEY(self.key.as_raw())`) without taking ownership; handles remain owned by the `windows-registry` `Key`.
7. CLI inspection commands (`inspect`, `dump`, `export`) read registry state and write only to stdout. They do not apply configuration changes.
8. The upstream WinSetView reference repository remains untouched and read-only.

No mutating APIs were found in production code. Keywords appearing in documentation do not constitute implementation violations. No mutating commands or APIs were executed during this audit.

## 10. Actual verification results

Baseline verification (commit `bf6da1c`):

- Registry unit tests: 118 passing
- Application unit tests: 16 passing
- Application integration tests: 7 passing
- Workspace tests: 164 passing, 11 ignored

The 11 live-registry integration tests are all marked `#[ignore = "requires Windows registry access"]` and do not run in CI. They require direct Windows registry access and are not claimed to run automatically in CI.

CI configuration:

- Windows job (`windows-latest`): `cargo check --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Cross-platform core job (`ubuntu-latest`, `macos-latest`): `cargo check -p winview-core`, `cargo test -p winview-core`.

Rust toolchain pinned to 1.100.0 via `rust-toolchain.toml`.

### Milestone 0.5.0 verification (this commit)

Rust 1.100.0 confirmed active via `rust-toolchain.toml`.

| Check | Result |
|---|---|
| `cargo check --workspace` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | 164 passing, 11 ignored |
| `cargo check -p winview-app --features gui` | **FAIL** — 3 compilation errors (GUI skeleton incompatible with eframe 0.37) |
| `cargo test -p winview-registry --test integration -- --ignored` | 11 passing, 0 failing |
| `cargo run -p winview-app -- inspect` | Exit code 0 (Complete) |
| `cargo run -p winview-app -- dump --format json` | Exit code 0 (Complete) |
| `cargo run -p winview-app -- export` | Exit code 0 (Complete) |

GUI compilation errors (not fixed in this milestone; Phase 1 will address):

```
error[E0407]: method `update` is not a member of trait `eframe::App`
error[E0046]: not all trait items implemented, missing: `ui`
error[E0308]: mismatched types in `CentralPanel::show`
```

All operations remain strictly read-only regarding Windows system configuration. No registry modifications were performed.

## 11. Known limitations

The following limitations exist and are classified as Phase 1 design constraints, deferred performance or reliability improvements, or future mutation-engine prerequisites. None have been resolved in this milestone.

- Registry enumeration still collects names and types into `Vec`s before downstream traversal limits apply.
- Individual registry payload reads can allocate memory before individual-value and snapshot-budget checks.
- Decoded values, duplicated representations, metadata, error records, and serialized output contribute additional memory usage.
- Depth-limit placeholders are not counted as actual key-open attempts and may increase snapshot size.
- Some recursive roots overlap, so the same underlying registry data may be observed more than once.
- Registry inspection is not an atomic point-in-time transaction. Concurrent registry modifications may affect observations.
- Native enumeration errors propagate, but entries collected before a midstream error are discarded under the existing `Result<Vec<T>>` contract.
- Invalid UTF-16 registry names are reported as errors rather than converted lossily.
- `CanonicalName` is retained as a decoded string, not a complete raw restoration record.
- FolderTypes inspects the configured default `TopViews` key, not every possible view definition.
- Windows version metadata detection may be incomplete. Complete inspection status does not guarantee complete version metadata.
- TOML cannot represent the entire unsigned QWORD range (values above `i64::MAX` fail TOML serialization).
- Snapshots are not guaranteed lossless restoration backups.
- The current implementation does not establish effective Explorer view precedence for every folder type.
- Full Windows 10/11 build compatibility has not been established.
- Some ignored legacy integration tests have limited assertions and are not proof of comprehensive live-registry coverage.
- The GUI is currently a feature-gated skeleton.

Classification:
- Phase 1 design constraints: GUI skeleton, effective view precedence, bounded/virtualized rendering, overlapping root deduplication.
- Deferred performance or reliability improvements: pre-limit `Vec` collection, upfront payload allocation, duplicate observation across overlapping roots, non-atomic observation, midstream entry discarding, lossy UTF-16 handling, full Windows 10/11 build compatibility, ignored integration test coverage.
- Future mutation-engine prerequisites: lossless restoration backups (TOML/QWORD range, `CanonicalName` as decoded string only), snapshot budget and atomicity, apply/restore/reset functionality.

## 12. Phase 1 entry requirements

The first GUI milestones should:

- Verify that the `gui` feature compiles.
- Provide an explicit GUI launch mechanism.
- Keep CLI behavior unchanged.
- Execute registry inspection outside the UI thread.
- Prevent overlapping inspection requests.
- Represent Idle, Loading, Complete, Partial, and Failed states.
- Show inspection status and recorded errors accurately.
- Display large datasets using bounded or virtualized rendering.
- Reuse shared `winview-core` inspection utilities.
- Remain strictly read-only.

These features are not implemented in this milestone.
