//! Storage migrations applied at startup.
//!
//! Each migration is idempotent — re-running on already-migrated data is a
//! no-op. New migrations are added as separate submodules. The current
//! convention is `v<schema-version>.rs` per the schema generation that
//! introduced the migration; the version refers to `RefactorPlan.md` phases,
//! not the schema `version` field on individual artifacts.

pub mod v2;
