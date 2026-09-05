//! Rebuild trigger for the default (non-`api-only`) build's embedded
//! `web/dist` copy (docs/DECISIONS.md #74, #88).
//!
//! `rust-embed`'s derive macro generates one `include_bytes!` per embedded
//! file, so rustc tracks *content* changes to files it already knows about,
//! but not files being added or removed — every `pnpm --filter web build`
//! run produces new content-hashed `_astro/*` filenames. Without this,
//! `cargo build` after a frontend rebuild can silently keep serving the
//! previous build's file list.
//!
//! Feature-gated on `api-only`'s *absence*: pointing `rerun-if-changed` at
//! `../web/dist` when building with `api-only` would make that build
//! (whose whole point is not depending on `web/dist`) re-run this script
//! whenever the directory happens to change, and fail outright on a fresh
//! checkout where it doesn't exist yet.
use std::path::Path;

fn main() {
    if std::env::var_os("CARGO_FEATURE_API_ONLY").is_some() {
        return;
    }

    println!("cargo::rerun-if-changed=../web/dist");

    if !Path::new("../web/dist").is_dir() {
        println!(
            "cargo::error=web/dist not found — the default build bakes the frontend into the \
             binary. Run `pnpm --filter web build` from the workspace root first, or build \
             with `--features api-only` for a build with no bundled frontend."
        );
    }
}
