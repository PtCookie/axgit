//! Rebuild trigger for the `embed-web` feature (docs/DECISIONS.md #74).
//!
//! `rust-embed`'s derive macro generates one `include_bytes!` per embedded
//! file, so rustc tracks *content* changes to files it already knows about,
//! but not files being added or removed — every `pnpm --filter web build`
//! run produces new content-hashed `_astro/*` filenames. Without this,
//! `cargo build --features embed-web` after a frontend rebuild can silently
//! keep serving the previous build's file list.
//!
//! Feature-gated: pointing `rerun-if-changed` at `../web/dist` unconditionally
//! would make every build re-run this script even when the directory is
//! irrelevant (default build) or absent (fresh checkout before the frontend
//! has ever been built).
fn main() {
    if std::env::var_os("CARGO_FEATURE_EMBED_WEB").is_some() {
        println!("cargo:rerun-if-changed=../web/dist");
    }
}
