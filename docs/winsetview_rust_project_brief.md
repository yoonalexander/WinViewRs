# Explorer View Manager for Windows: Rust Reimplementation of WinSetView

**Status:** Project brief and implementation roadmap  
**Target:** Windows 10 and 11  
**Language:** Rust  
**Goal:** Build a polished, native Rust application that reproduces the core functionality of WinSetView, emphasizing safe configuration changes, explicit change previews, backup/restore, and testability.

> **Instructions for the coding agent:** Treat this document as the initial specification and plan, not an invitation to modify the user's live Explorer settings immediately. Inspect the upstream code and manual first. Implement read-only discovery and tests before any destructive registry operations. Ask for permission before applying changes to an actual user system, restarting Explorer, or running privileged commands. Do not assume undocumented registry semantics or feature compatibility.

## 1. Upstream project and legal obligations

- **Original application:** [WinSetView](https://github.com/LesFerch/WinSetView)
- **Original source:** [WinSetView/src](https://github.com/LesFerch/WinSetView/tree/main/src)
- **Original license:** [MIT License](https://github.com/LesFerch/WinSetView/blob/main/LICENSE)
- **Official manual:** [WinSetView Manual](https://github.com/LesFerch/WinSetView/blob/main/Manual.md)
- **Requirements:** [System Requirements](https://github.com/LesFerch/WinSetView/blob/main/System-Requirements.md)
- **Original author:** Les Ferch

### Can the source be copied and rewritten?

**Yes.** The upstream repository declares the MIT License, which permits use, copying, modification, distribution, sublicensing, and sale, subject to its conditions. For copies or substantial portions of the software, preserve the upstream copyright notice and MIT permission notice. It is reasonable to add a new copyright notice for original Rust contributions. Reuse of third-party assets or dependencies may require additional license checks. The MIT license does not automatically grant rights to trademarks, project branding, or third-party material.

Suggested README attribution:

> This project is a Rust reimplementation of [WinSetView](https://github.com/LesFerch/WinSetView), originally developed by Les Ferch. The original project is licensed under the MIT License. This implementation uses a redesigned Rust architecture and is an independent community project, not affiliated with or endorsed by the original author.

**Repository choice:** Prefer creating a separate Rust repository for a clean architecture and clear portfolio history. Keep a local upstream clone for reference. Preserve license notices for adapted code and document which upstream behaviors are implemented. A GitHub fork is also legally and technically valid.

**Commands:**

```powershell
git clone https://github.com/LesFerch/WinSetView.git
cargo new explorer-view-rs
cd explorer-view-rs
```

The suggested working name `explorer-view-rs` is provisional. Avoid implying that the project is the official WinSetView release.

## 2. Why this project exists

WinSetView is an established Windows utility for globally configuring File Explorer folder views. It allows users to set default view layouts, columns, sort/group settings, and behavior for different folder **types**, with backup and restore functions. The proposed Rust version is **not** inherently better simply because it uses Rust: registry operations and Explorer refreshes are not necessarily performance bottlenecks. The value proposition is:

1. A cohesive native Rust application with a maintainable codebase.
2. Explicit, inspectable, safely reversible registry changes.
3. Configuration profiles that can be validated, compared, and shared.
4. Testable separation of Windows-specific operations and pure business logic.
5. Modernized UX, useful diagnostics, and practical Windows installers.

**Portfolio demonstration:** Rust systems development, Windows registry and Explorer internals, error handling, data modeling, integration testing, safe system modification, packaging, and technical documentation.

## 3. What upstream WinSetView actually does

Upstream WinSetView does **not** scan every directory and individually assign its appearance. It adjusts registry-based defaults that Explorer consults when folders are opened. Its workflow can clear saved folder views, set defaults by folder type, optionally change other Explorer options, and restart Explorer.

The upstream documentation describes:

- Default layouts for all supported Explorer folder types, not only General Items / Documents / Pictures / Music / Videos, but also Downloads, libraries, search results, OneDrive, and some file dialogs.
- Modes such as Details, List, and multiple icon sizes.
- Configurable columns, column order and widths, sorting, and grouping.
- Folder-type inheritance from global settings.
- Auxiliary Explorer options and Windows-version-specific behavior.
- INI-based portable configuration and command-line operation.
- Backup/restore of relevant registry values, with documented limits.
- Applying changes through a PowerShell script, then restarting Explorer.

**Important:** Windows may prioritize previously saved per-folder views over type defaults. WinSetView may clear those saved views, potentially deleting individually customized layouts. Backups and restore are therefore **core requirements**, not polish.

### Relevant Windows registry areas

These paths were observed in upstream `src/WinSetView.ps1` and are **research pointers only**, not a blanket instruction to delete/write them:

```text
HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\BagMRU
HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags
HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\Bags\AllFolders\Shell
HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Streams
HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Streams\Defaults
HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes
HKLM\Software\Microsoft\Windows\CurrentVersion\Explorer\FolderTypes
HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced
```

Other keys are used for optional Explorer settings. Research the meaning, format, scope, data types, and compatibility of **each** key/value before implementing mutations. Maintain an explicit allowlist of supported changes.

### Simplified Explorer configuration precedence

1. Previously saved per-folder preferences (`Bags`, `BagMRU`) may override type defaults.
2. Per-user folder-type view defaults and streams can affect the result.
3. Machine-level Windows `FolderTypes` definitions provide baseline defaults.

This is conceptual, not a complete or guaranteed Windows internals specification. Verify actual behavior on supported Windows builds.

## 4. Upstream implementation inventory

The source directory contains these key files:

| Upstream file | Role | Proposed Rust rewrite |
|---|---|---|
| `src/WinSetView.htm` | HTML/JScript main UI | `egui`/`eframe` native UI |
| `src/Modal.htm` | Modal dialogs | Native dialogs and confirmation flows |
| `src/WinSetView.ps1` | Core configuration and application orchestration | Typed Rust configuration and apply engine |
| `src/CSReg.cs` | Registry helper supporting import/export/query/add/delete | Safe Rust registry abstraction |
| `src/CloseExplorerWindows.cs` | Explorer window helper | Deliberate Explorer refresh/restart module |
| `src/GetMoreProperties.cs` | Property-discovery helper | Rust Windows API integration where needed |
| `src/WinSetView.iss` | Installer configuration | New Windows packaging setup |

Upstream also contains documentation, icons, and assets. Check asset-specific rights before copying.

**Upstream environment:** The original supports Windows 7/8/10/11 and uses PowerShell, .NET, MSHTML, and JScript, according to its requirements. Our initial scope is Windows **10/11** only. Full parity with the original is **not** an MVP requirement.

## 5. Proposed Rust architecture

```text
Native Rust desktop UI (egui / eframe)
              |
              v
Typed settings and profile model (serde)
              |
              v
Validation + deterministic change planner
              |
      +-------+-------+
      |               |
      v               v
Windows registry   Snapshot + restore
adapter            engine
      |               |
      +-------+-------+
              |
              v
Apply changes + verification + optional Explorer refresh
```

### Recommended technologies

| Area | Suggested tool | Notes |
|---|---|---|
| Language | Rust, stable toolchain | Native Windows target |
| GUI | `egui` and `eframe` | UI authored in Rust |
| Windows APIs | `windows` | Win32/COM integration when required |
| Registry | `windows-registry` or carefully chosen Win32 bindings | Preserve registry value types; verify crate features and APIs |
| Configuration | `serde`, `serde_json` | Versioned profile schema |
| Error handling | `thiserror` for domain errors, `anyhow` for app-level context | Actionable messages |
| Diagnostics | `tracing` | Redact personally identifying paths where appropriate |
| Testing | `cargo test`, isolated Windows integration tests | Prefer mockable adapters and VM tests |
| CI | GitHub Actions | Build/test Windows executables |
| Packaging | Windows installer or portable zip | Prefer a working portable release first |

### Suggested modules

```text
src/
  main.rs                 # App startup
  ui/                     # Native UI screens and dialogs
  model/                  # Folder types, profiles, view settings
  profiles/               # Import/export/versioning/validation
  planner/                # Diff and dry-run operations
  windows/
    registry.rs           # Narrow, typed read/write adapter
    explorer.rs           # Explorer status and refresh coordination
    detection.rs          # Windows version/capability discovery
  backup/                 # Snapshots, metadata, restore verification
  apply/                  # Validated execution and recovery flow
  diagnostics/            # Structured logs and troubleshooting
 tests/
   fixtures/              # Synthetic registry/profile fixtures
   integration_windows.rs
 LICENSE
 README.md
```

Core architectural invariant: **UI never writes registry values directly.** UI edits a typed proposed configuration; planner calculates and validates changes; backup captures before-state; apply executes authorized operations; verification confirms outcomes; recovery can restore previous state.

## 6. Functional scope

### MVP (v0.1)

- [ ] Detect whether the host is supported Windows 10 or Windows 11.
- [ ] Read current relevant Explorer view settings without writing anything.
- [ ] Show known folder types and current/default view information where confidently detectable.
- [ ] Configure at least one folder-type default, starting with Downloads.
- [ ] Support a small set of views: Details, List, and at least one icon mode.
- [ ] Support toggling the Downloads grouping behavior if verified on test builds.
- [ ] Calculate and display the exact registry change plan (add/update/remove with scope).
- [ ] Back up every affected value/key before mutation, including value types and metadata needed for recovery.
- [ ] Provide confirmation before apply, especially if clearing saved folder views or restarting Explorer.
- [ ] Apply the configuration on a disposable Windows environment and verify actual Explorer behavior.
- [ ] Restore from the snapshot and verify recovery.
- [ ] Document known limitations and how to recover manually if the app fails.

### Later releases (v0.2+)

- [ ] General Items, Documents, Pictures, Music, Videos, and more folder types.
- [ ] Global defaults with per-folder-type overrides and inheritance.
- [ ] Column inclusion, order, widths, property identifiers.
- [ ] Sorting and grouping, including multiple attributes where supported.
- [ ] Import/export reusable JSON profiles and show profile differences.
- [ ] Additional Explorer options, each with independent privilege/risk labeling.
- [ ] Command-line dry-run / apply / restore (only after safety path is proven).
- [ ] Wider Windows build compatibility, installer, localization, release packaging.

### Deliberately out of scope for v0.1

- Full feature parity with upstream WinSetView.
- Windows 7 or Windows 8 support.
- Arbitrary user-specified registry editing.
- Background monitoring, continuously running services, or Explorer injection.
- Silent destructive changes or undocumented machine-wide modifications.

## 7. Safety requirements (non-negotiable)

1. **Read-only first.** Do not perform registry changes in Phase 1.
2. **Disposable environment.** Test writes and Explorer restarts first in a Windows VM or disposable test machine, not the primary workstation.
3. **Snapshot before mutation.** Preserve all affected entries and registry data types, not just reconstructed defaults.
4. **Dry-run diff.** Show key, value, prior state, desired state, and whether an operation deletes any existing data.
5. **Narrow allowlist.** Reject writes outside explicitly supported registry paths and value shapes.
6. **Preserve unrelated settings.** Avoid resetting Explorer options not selected by the user.
7. **Explicit confirmation.** Warn when clearing saved per-folder preferences, terminating processes, closing windows, or restarting Explorer.
8. **Recoverable failures.** Design for interrupted or partial apply. Verify backup readability before beginning changes and report partial failures clearly.
9. **Least privilege.** Prefer HKCU changes that do not require administrator access; request elevation only for individually identified elevated operations.
10. **Version-aware operations.** Do not assume all keys and behaviors are identical across Windows 10/11 builds.
11. **Honest verification.** A successful registry write is **not** the same as verified Explorer UI behavior; check both.
12. **No unsupported guarantees.** If registry updates are not fully atomic, do not call the operation a true database-style transaction. Use best-effort rollback plus diagnostics.

## 8. Incremental development roadmap

### Phase 0: Reconnaissance and project setup

**Tasks**
- Clone upstream for reference.
- Read `README.md`, `README-more.md`, `Manual.md`, `System-Requirements.md`, relevant source files, and license.
- Produce a feature matrix: upstream functionality / desired Rust functionality / implementation complexity / risk / test strategy.
- Identify registry reads/writes and control flow in `WinSetView.ps1`; flag destructive paths.
- Initialize the Rust application, CI, license notices, README, and basic UI skeleton.

**Done when:** The desktop app launches, builds on Windows, and a traceable feature/registry-operation inventory exists. No registry writes have been implemented.

### Phase 1: Read-only inspection

**Tasks**
- Implement platform/version detection.
- Implement typed registry reads with consistent errors for missing/unsupported keys.
- Discover relevant folder types and present a readable inspector.
- Build synthetic fixtures that cover absent keys, values of different registry types, and Windows-version differences.

**Done when:** Running the program never changes registry state and inspector output is cross-checked against known system settings and the upstream behavior.

### Phase 2: Snapshot and restore foundation

**Tasks**
- Implement snapshots of all planned affected registry state.
- Include timestamp, schema version, affected path inventory, and checksums/verification metadata where useful.
- Implement restore into **isolated test registry locations first**, then disposable Windows instances.
- Test failure handling (missing keys, inaccessible keys, partial snapshot, interrupted restoration).

**Done when:** Backup/restore round trips pass integration tests and the app can clearly identify an invalid or incomplete backup.

### Phase 3: Single real feature

**Tasks**
- Implement a configuration model for Downloads default view/grouping.
- Create a planner that outputs human-readable diffs.
- Apply only supported, validated operations in a disposable Windows installation.
- Confirm the actual result in File Explorer after the appropriate refresh.
- Restore and verify original behavior.

**Done when:** A Downloads view change can be previewed, applied, verified, and undone with no unexplained collateral changes.

### Phase 4: General folder-type profiles

**Tasks**
- Add common folder types, view modes, inheritance, and safe presets.
- Add a validated, versioned JSON configuration schema.
- Separate capabilities supported on one Windows version from unsupported ones.

**Done when:** Common folder-type views work across representative test environments, and profile errors are caught before any registry write.

### Phase 5: Advanced controls

**Tasks**
- Implement columns, ordering, grouping, sorting, and widths with documented property identifiers.
- Add additional folder types incrementally.
- Provide precise UI explanations for options with potentially surprising effects.

**Done when:** Every supported option has a regression test and an observed Explorer behavior test where feasible.

### Phase 6: Release hardening

**Tasks**
- Add usable presets, error reporting, a troubleshooting guide, and a recovery walkthrough.
- Produce a portable build and optional installer.
- Add GitHub Actions Windows builds, release tagging, and changelogs.
- Document differences from upstream and unsupported scenarios.

**Done when:** A new Windows user can safely understand, install/run, preview, apply, and reverse a supported setting without manually editing the registry.

## 9. Testing and quality plan

**Unit tests (safe, cross-platform when possible):**
- Profile parsing and validation.
- Stable, deterministic change-plan generation.
- No-op detection when desired values already match current values.
- Correct handling of missing values, alternate registry types, and unsupported folder identifiers.
- Snapshot serialization/deserialization and integrity checks.

**Windows integration tests:**
- Registry reads using synthetic test keys under an isolated subtree.
- Registry writes and round-trip restoration on those isolated keys.
- End-to-end registry and Explorer behavior only in a disposable Windows environment.
- Failure recovery when a planned operation cannot complete.

**Manual acceptance checks:**
- The original system's customized folder views are preserved in the initial backup.
- The user can see every planned deletion or reset before proceeding.
- The user can cancel without changing registry state.
- Explorer is never restarted unexpectedly.
- A previously applied configuration can be restored and its outcome checked.
- Unsupported Windows builds produce an informative warning rather than speculative changes.

## 10. Suggested first coding-agent task

Implement **only Phase 0 and the read-only skeleton of Phase 1**:

1. Inspect the upstream WinSetView manual and source for folder-type settings and registry behavior.
2. Create a new Rust project targeting Windows 10/11 using `egui`/`eframe`.
3. Build a small UI with a header, Windows compatibility status, and a read-only list of discovered Explorer folder types.
4. Implement a registry adapter exposing typed **read** operations only.
5. Add mocked fixtures and unit tests for the discovery and configuration model.
6. Include the upstream MIT license notice and appropriate attribution in README.
7. Output a concise implementation report identifying uncertain registry semantics, exact APIs used, test results, and next steps.

**Explicitly do not implement in this first task:** registry modification, deletion of Bags/BagMRU, importing `.reg` files, Explorer restart, process termination, or administrator elevation.

### Completion criteria for the first agent task

- `cargo fmt --check` succeeds.
- `cargo clippy` passes without unaddressed warnings (or clearly documented exceptions).
- `cargo test` passes.
- The UI opens on supported Windows, with clear non-destructive error handling elsewhere.
- No code path performs registry writes.
- Project attribution and licensing are present.

## 11. Portfolio positioning

**Possible project description:**

> Native Rust utility for managing Windows File Explorer folder-view defaults. Reimplements the core behavior of WinSetView with typed Windows registry integration, dry-run change planning, automatic backups, restore verification, and portable configuration profiles.

Do not claim performance gains, complete feature parity, or reliability metrics without measuring and validating them. A high-quality, demonstrably reversible v0.1 is more impressive than an ambitious but unsafe rewrite.

## Appendix A: Upstream MIT License notice

The upstream license file currently contains the following text. Retain it when copying or adapting substantial upstream material (and inspect dependencies/assets separately):

```text
MIT License

Copyright (c) 2021 Les Ferch

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

---

**Research basis:** Inspection of the linked WinSetView GitHub repository, its main branch, MIT license, README, manual, system requirements, and source directory in November 2026. Implementation details should be verified against the upstream version actually cloned and the target Windows builds. This document is an engineering plan, not legal advice.
