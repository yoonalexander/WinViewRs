# Phase 1 Completion Report

WinViewRs — Phase 1 Exit Audit (Milestone 1.9a)

## 1. Baseline and review status

- Audited commit: `3a36b33fa84ccc8b26497fbc83ff07dafcbca44d` ("Milestone 1.8b: Finalize inspector usability and presentation").
- Branch: `main`.
- Working tree at audit start: clean.
- Rust toolchain: `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, pinned by `rust-toolchain.toml`.
- Overall review status: Implementation complete; automated verification passed; manual interactive GUI acceptance remains PENDING (no interactive Windows desktop session was available during this audit).

This audit is verification and documentation only. No production Rust code, tests, dependencies, Cargo manifests, or CI configuration were modified.

## 2. Workspace architecture

WinViewRs is a virtual Cargo workspace of three crates with a strict one-directional dependency flow (`winview-app` → `winview-core` + `winview-registry`; `winview-registry` → `winview-core`; `winview-core` depends on neither).

### Core layer (`winview-core`, cross-platform)

- Platform-neutral inspection models in `model.rs`: `InspectionSnapshot`, `SnapshotMetadata`, `InspectionStatus` (Complete/Partial/Failed), `FolderTypesSnapshot`, `FolderTypeSnapshot`, `ViewSnapshot`, `KeySnapshot`, `RawRegistryValue`, `RegistryValue`, `DecodingStatus`, `ReadError`, `ValueReadResult`, `InspectionError`, `ObservationSource`, `DetectionMethod`.
- Read-only registry interfaces in `registry/traits.rs`: `RegistryReader` (`open_key`, `enum_keys`, `get_value`, `get_value_type`, `inspect_all`) and `RegistryKey` (`enum_values`, `enum_subkeys`, `get_value`, `get_value_type`, `open_subkey`). Both expose read operations only.
- Shared inspection utilities in `inspection.rs`: `count_snapshot_read_errors`, `count_key_read_errors`, `count_folder_types_read_errors`, `snapshot_has_successful_observations`, and status finalization helpers.
- Version model in `windows/version.rs` and `windows/features.rs`: `VersionInfo`, `DetectionMethod`, `FeatureCapabilities`, `SupportTier`, `support_tier`.

### Registry layer (`winview-registry`, Windows-only)

- `WindowsRegistryReader` in `reader.rs` implements `RegistryReader`/`RegistryKey` using the `windows-registry` crate plus `RegEnumKeyExW`/`RegEnumValueW` over borrowed handles (`HKEY(self.key.as_raw())`), never taking ownership.
- Windows version detection via `detect_windows_version` (`RtlGetVersion` with registry fallback).
- Inspection domains: HKCU/HKLM FolderTypes with default TopViews, Bags/BagMRU/AllFolders, Streams (+Defaults/ Desktop), Advanced (fixed 7 supported values), 11 CLSID override paths, and HKCU/HKLM Explorer policies.
- Raw value preservation: raw bytes always captured; decoding into `RegistryValue` for REG_SZ/EXPAND_SZ/DWORD/QWORD/BINARY/MULTI_SZ; `Malformed`/`UnknownType` preserve raw bytes with `decoded = None`.
- Optional vs required key policies via `KeyPresence`; HKCU FolderTypes/Advanced tolerate absence, HKLM FolderTypes open failure is recorded, recursive children are `Required`.
- Resource limits: depth 32, 50,000 key-open attempts per domain (shared across HKCU/HKLM FolderTypes), 1,000 values per generic key, 1 MiB per value, 50 MiB shared captured-payload budget.

### Application layer (`winview-app`, Windows-only)

- CLI (`main.rs`, `cli/`): `inspect` (human summary), `dump --format json` (pretty JSON), `export` (TOML, lossy), and feature-gated `gui`. Exit codes: 0 Complete, 1 Failed/operational, 2 invalid args/format, 3 Partial; serialization/I/O failures take precedence.
- Feature-gated egui/eframe GUI (`gui/`, `gui` feature, off by default; minimum window 640x400, default 960x640).
- Background inspection worker (`gui/inspection.rs`) on thread `winview-inspection` with Idle/Loading/Finished/Error states; rejects overlapping requests; nonblocking `poll()`.
- Shared captured snapshot ownership via `Arc<InspectionSnapshot>` in `CapturedInspection`; GUI browsers borrow through deref coercion.
- Overview browser (`gui/overview.rs`), FolderTypes browser (`gui/folder_types.rs`, hive separation + pagination + raw details), Advanced settings browser (`gui/advanced.rs`), Registry domains browser (`gui/registry_browser.rs`, hierarchical navigation + pagination), shared raw-value details (`gui/value_details.rs`), scoped recorded diagnostics (`gui/diagnostics.rs`, paginated).
- Background JSON file export (`gui/export.rs`) on thread `winview-export`, reusing the CLI serializer, create-new semantics, explicit absolute `.json` destination, progress/result shown outside a collapsed panel.

The upstream reference repository at `../WinSetView-reference` remains strictly read-only and was not modified.

## 3. Phase 1 milestone requirements matrix

| Milestone | Required outcome | Implementation | Automated evidence | Manual | Known limitations |
|---|---|---|---|---|---|
| 1.1 | Native feature-gated GUI foundation | Implemented | `cargo check/clippy/test -p winview-app --features gui` PASS; release build PASS | PENDING | No interactive launch verified in audit |
| 1.2 | Responsive background inspection | Implemented | `inspection_*` worker tests PASS (off-thread, overlap rejection, retry, panic) | PENDING | Non-atomic observation |
| 1.3 | Overview and captured metadata | Implemented | `overview_*` tests PASS (4) | PENDING | Metadata may be incomplete |
| 1.4 | FolderTypes browsing and hive separation | Implemented | `folder_*` tests PASS (HKCU/HKLM separation, pagination, selection) | PENDING | Default TopViews only |
| 1.5 | Advanced settings inspection | Implemented | `advanced_*` tests PASS (7 settings, statuses) | PENDING | Fixed supported value list |
| 1.6 | Hierarchical registry domain browsing | Implemented | `registry_*` tests PASS (domains separate, nav resets, pagination) | PENDING | Root shows "No key selected" |
| 1.7 | Bounded raw values and scoped diagnostics | Implemented | `value_details_*`, `diagnostic_*`, raw-detail tests PASS | PENDING | Preview truncation |
| 1.8 | Background snapshot export and UI usability | Implemented | `json_export_*`, `export_worker_*` tests PASS (5) | PENDING | Non-atomic; no file dialog |
| 1.9 | Exit audit and acceptance verification | Implemented (this document) | All automated commands PASS | PENDING | Manual acceptance outstanding |

Interactive behavior is not marked verified merely because egui-context unit tests pass.

## 4. Read-only safety audit

Searches run across `crates` for mutation and process APIs (ripgrep was unavailable; the Kilo `grep` tool over `*.rs` was used):

- `RegistryWriter|RegSetValue|RegCreateKey|RegDelete|RegRenameKey|RegRestoreKey|RegLoadKey|RegSaveKey|RegReplaceKey|RegCopyTree` → No matches.
- `set_value|create_key|delete_value|remove_value|delete_key|remove_tree|set_u32|set_u64|set_string|set_bytes|set_multi_string` → No matches.
- `Stop-Process|taskkill|TerminateProcess|runas|elevate|ShellExecute|Command::new` → 8 matches, all `Command::new` in `crates/winview-app/tests/cli_tests.rs` (test-only subprocess invocation of the built binary). No production process spawning, elevation, or Explorer termination.
- `RegOpenKey|RegQueryValue|RegCloseKey|RegFlushKey|RegNotifyChange` → No matches (no low-level registry mutation/open calls).

Reviewed code paths:

- `RegistryReader`/`RegistryKey` traits expose only read methods; no write/set/delete/create/restore/load/save/replace/copy.
- `WindowsRegistryReader::inspect_all` and every `inspect_*` domain function only open, enumerate, and read values.
- CLI handlers (`inspect`, `dump`, `export`) read registry state and write only to stdout.
- GUI actions and worker entry points perform inspection and JSON file export only.
- JSON export (`export_snapshot_json`) uses `OpenOptions::new().write(true).create_new(true)` — create-new semantics, never overwriting existing files; no parent-directory creation.

Conclusion: No mutating registry APIs or destructive system operations exist in production code. No mutating API was executed during this audit. The `windows` crate features enabled are limited to `Wdk_System_SystemServices`, `Win32_Foundation`, `Win32_System_Registry`, `Win32_System_SystemInformation` (read/enumeration only).

## 5. Automated verification

All commands executed at commit `3a36b33`:

| Command | Result |
|---|---|
| `cargo check --workspace` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (no warnings) |
| `cargo test --workspace` | PASS — 165 passed, 11 ignored |
| `cargo check -p winview-app --features gui` | PASS |
| `cargo clippy -p winview-app --features gui --all-targets -- -D warnings` | PASS (no warnings) |
| `cargo test -p winview-app --features gui` | PASS — 62 unit, 8 integration |
| `cargo build --release -p winview-app --features gui` | PASS (release profile) |

Default application unit tests: 16 passing. GUI application unit tests: 62 passing. Application integration tests: 8 passing. Registry unit tests: 118 passing. Core lib tests: 4; model 11; property 3; version 5.

The 11 ignored live-registry integration tests are all `#[ignore = "requires Windows registry access"]` and do not run in CI.

`cargo-audit` and `cargo-deny` are not installed (exit 101); no dependency-security verification was performed and none is claimed.

CI configuration (`.github/workflows/ci.yml`):

- `windows` job (`windows-latest`, toolchain 1.100.0): `cargo check --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo check -p winview-app --features gui`, `cargo clippy -p winview-app --features gui --all-targets -- -D warnings`, `cargo test -p winview-app --features gui`.
- `core-cross-platform` job (`ubuntu-latest`, `macos-latest`): `cargo check -p winview-core`, `cargo test -p winview-core`.

## 6. CLI acceptance verification

Executed against the built debug binary (cargo build progress on stderr suppressed; exit codes captured from the process):

| Command | Outcome | Exit code |
|---|---|---|
| `--help` | Lists inspect/dump/export/help; no gui in default build | 0 |
| `--version` | `winview-app 0.1.0` | 0 |
| `--features gui -- --help` | Adds `gui` command | 0 |
| `--features gui -- gui --help` | "Launch the read-only desktop inspector" | 0 |
| `inspect` | Status: Complete | 0 |

- Default CLI commands remain available; GUI support is feature-gated (absent from the default build's command list, present under `--features gui`).
- No mutation-related commands exist.
- JSON dump (`dump --format json`) written to a unique temporary path: parses successfully; contains `metadata`, `status` (state `Complete`), inspection domains, and `errors`; process exit code 0.
- TOML export (`export`) written to a unique temporary path: serialization succeeded; process exit code 0. This snapshot contained no unsigned QWORD above `i64::MAX`; the documented TOML limitation (values above `i64::MAX` fail TOML serialization) remains in effect.

No personal registry paths, raw payloads, or private system information are reproduced in this report.

The 11 ignored live-registry integration tests were inspected and confirmed strictly read-only (only `open_key`, `get_value`, `inspect_all`). Executed separately: 11 passed, 0 failed. These are reported separately from the standard automated suite.

## 7. GUI behavior and resource limits

Bounded presentation limits (verified in source):

- 25 folder records per page (`folder_types.rs`).
- 25 registry keys or values per page (`registry_browser.rs`).
- 25 recorded diagnostics per page (`diagnostics.rs`).
- 64-byte hexadecimal previews (`value_details.rs`).
- 160-character value text previews (`value_details.rs`, `folder_types.rs`, `registry_browser.rs`).
- Eight decoded multi-string entries (`value_details.rs`).
- 512-character diagnostic text previews (`diagnostics.rs`).

These bound GUI presentation only. They do not cap total application memory consumption: the full snapshot is held in memory (shared via `Arc`), and registry enumeration may collect names/types into `Vec`s before downstream limits apply (see limitations).

## 8. Snapshot export

- Shared immutable snapshot ownership: `CapturedInspection.snapshot: Arc<InspectionSnapshot>`; the export worker receives an `Arc` clone, not a deep copy.
- Background serialization and file writing on thread `winview-export`; GUI remains responsive; nonblocking polling; overlapping export requests rejected.
- Explicit absolute `.json` destination required; no default destination; create-new semantics (`create_new(true)`) so existing files are never overwritten.
- Supports Complete, Partial, and Failed captured snapshots (the complete snapshot is serialized).
- Error reporting preserves the full error chain (contextual message plus underlying filesystem/serialization error).
- Privacy: exports may contain personal paths and system-specific data; users are warned to inspect/redact.
- Non-atomic: a failed or interrupted write may leave an incomplete new file; export is not crash-safe and is not a restorable registry backup.

## 9. Known limitations

Retained Phase 0 limitations (not newly resolved):

- Registry enumeration collects names/types into `Vec`s before downstream limits apply.
- Individual payload reads can allocate memory before value/snapshot-budget checks.
- Decoded values, duplicated representations, metadata, errors, and serialized output add memory usage.
- Depth-limit placeholders are not counted as key-open attempts and may increase snapshot size.
- Overlapping recursive roots may observe the same data more than once.
- Inspection is not an atomic point-in-time transaction.
- Midstream enumeration errors discard already-collected entries under the `Result<Vec<T>>` contract.
- Invalid UTF-16 names are reported as errors, not lossily converted.
- `CanonicalName` is retained as a decoded string, not a complete raw restoration record.
- Only the configured default `TopViews` key is inspected.
- Windows version metadata may be incomplete; Complete status does not guarantee complete metadata.
- TOML cannot represent unsigned QWORD values above `i64::MAX`.
- Snapshots are not guaranteed lossless restoration backups.
- No effective Explorer view precedence analysis is established.
- Full Windows 10/11 build compatibility has not been validated.
- Some ignored legacy integration tests have limited assertions.

Phase 1 limitations:

- Manual interactive GUI acceptance remains outstanding (PENDING) — not performed in this audit.
- No native file dialog; the export destination is typed manually.
- GUI previews are truncated (see resource limits); the raw preview is not a complete payload dump.
- No configuration editing of any kind.
- GUI export is non-atomic; interrupted writes may leave incomplete files.

## 10. Manual Windows acceptance checklist

To be executed by the user on an interactive Windows desktop. Every item is PENDING.

- [ ] PENDING — Application launches; minimum window size (640x400) remains usable.
- [ ] PENDING — Run inspection and Refresh inspection work; controls disable during inspection/export.
- [ ] PENDING — Overview shows status, error count, and Windows metadata.
- [ ] PENDING — FolderTypes shows separate HKLM (system) and HKCU (user) records.
- [ ] PENDING — FolderTypes pagination label matches displayed records; value selection shows raw details.
- [ ] PENDING — Advanced settings show 7 supported values with raw value details.
- [ ] PENDING — Registry domains support nested navigation (Roots/Up) and value inspection.
- [ ] PENDING — Diagnostic panels paginate and preserve scope (hive/folder/key).
- [ ] PENDING — Empty and unavailable observations are shown distinctly (not fabricated).
- [ ] PENDING — JSON export to a new file succeeds; progress visible with panel collapsed.
- [ ] PENDING — Existing-file overwrite is rejected with a useful error.
- [ ] PENDING — Invalid export destinations (relative, wrong extension, missing parent) produce understandable errors.
- [ ] PENDING — Collapsing the export panel does not hide progress or block completion.
- [ ] PENDING — Refresh becomes available after export finishes.
- [ ] PENDING — Registry domain roots show "No key selected" (not a misleading zero).
- [ ] PENDING — Application closes safely.

## 11. Phase 2 prerequisites

Registry mutation features are not authorized and are not implemented. Any future mutation capability requires a separate approved design covering: typed configuration; validation; deterministic change planning with dry-run diffs; backup and recovery; rollback; Windows version/build compatibility checks; and testing in a disposable Windows VM before any host use. No such features were started.

## 12. Exit decision

IMPLEMENTATION COMPLETE, MANUAL ACCEPTANCE PENDING.

All required automated verification passed at commit `3a36b33` with no warnings or formatting failures, and the read-only safety audit found no production mutation paths. Interactive Windows GUI acceptance has not been performed and remains PENDING; therefore Phase 1 is not marked COMPLETE.