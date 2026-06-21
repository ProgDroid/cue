# cue backend-foundation — SDD progress ledger

Plan: docs/superpowers/plans/2026-06-21-cue-backend-foundation.md
Branch: feat/backend-foundation

## Tasks
- Task 1: pending — Cargo scaffold + config
- Task 2: pending — SQLite schema migration + pool
- Task 3: pending — domain models + DTO
- Task 4: pending — dev seed + seeder
- Task 5: pending — catalogue assembly + GET /api/catalogue
- Task 6: pending — SPA static serving + bootstrap
- Task 7: pending — Docker packaging

## Minor findings (for final review triage)
(none yet)

## Decisions
- Crate is lib (`src/lib.rs`, `pub mod` per module) + thin binary (`src/main.rs` uses `cue::`). Adopted in Task 1 to satisfy `cargo test --lib`; plan updated for Tasks 2-6.
