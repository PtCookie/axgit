//! Smart HTTP (clone/fetch only) — skeleton, no implementation yet.
//!
//! Planned shape (docs/ARCHITECTURE.md "Smart HTTP"):
//!
//! - `GET /{repo}.git/info/refs?service=git-upload-pack` — spawn
//!   `git upload-pack --stateless-rpc --advertise-refs {repo}` and prepend
//!   the pkt-line service header to its output.
//! - `POST /{repo}.git/git-upload-pack` — stream the request body (gunzip
//!   when `Content-Encoding: gzip`) into `git upload-pack --stateless-rpc`
//!   stdin and stream stdout back as the response.
//! - receive-pack must never be exposed in any form: every
//!   `git-receive-pack` request is answered with 403 `read_only`. The
//!   rejection is wired up in `crate::routes` alongside the stubs.
