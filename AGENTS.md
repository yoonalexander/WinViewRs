# WinViewRs: Agent Instructions

## Project Overview

WinViewRs is a native Windows application written in Rust.

It is a reimplementation of WinSetView by Les Ferch:

https://github.com/LesFerch/WinSetView

Read `docs/winsetview_rust_project_brief.md` before planning or implementing features.

## Repository Structure

- `WinViewRs/`: Main Rust project. All new code goes here.
- `WinSetView-reference/`: Original WinSetView implementation, located in the sibling directory.

The reference repository is strictly read-only. Never modify its files.

## Engineering Standards

- Use Rust for the core application and Windows integration.
- Prefer modular architecture and strongly typed interfaces.
- Follow idiomatic Rust conventions.
- Keep changes small, testable, and well documented.
- Do not introduce dependencies without justification.
- Use test-driven development where appropriate.
- Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` after implementation.
- Do not claim tests passed unless they were executed successfully.

## Safety Requirements

- Phase 0 and Phase 1 must remain read-only regarding Windows system configuration.
- Never modify or delete Windows registry keys during these phases.
- Never restart or terminate Windows Explorer without explicit approval.
- Never execute destructive system commands.
- Future registry-writing features must include backup, validation, and rollback mechanisms.
- Test system modifications in a disposable Windows virtual machine before using them on a host system.

## Agentic Development Workflow

1. Plan before implementing.
2. Use read-only research subagents to investigate the original implementation.
3. Define shared interfaces before splitting implementation tasks.
4. Ask for approval before major architecture changes.
5. Delegate independent work to separate agents when appropriate.
6. Use separate Git worktrees for parallel coding tasks.
7. Do not allow multiple agents to edit the same files simultaneously.
8. Review and test all changes before merging.
9. Commit completed milestones with descriptive messages.
10. Report completed work, tests, assumptions, and remaining issues.

## Licensing

WinSetView is licensed under the MIT License.

Preserve the original copyright and license notices for reused or adapted material.

Clearly attribute Les Ferch and distinguish the Rust implementation from the original project.

## Current Milestone

Phase 0: Complete. Baseline established in `docs/PHASE_0_COMPLETION.md`.

Phase 1: Read-only desktop GUI — implementation complete. Automated verification passed at commit `3a36b33`; interactive manual acceptance is pending. Exit audit recorded in `docs/PHASE_1_COMPLETION.md`.

Phase 2: Not authorized. No registry mutation, apply, restore, or reset functionality is permitted until a separate design is approved.

No registry modifications are authorized.