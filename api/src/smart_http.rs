//! Smart HTTP (clone/fetch only) — proxies `git upload-pack --stateless-rpc`
//! (api/README.md "Smart HTTP", docs/DECISIONS.md #13).
//!
//! - `GET /{repo}.git/info/refs?service=git-upload-pack` — spawns
//!   `git upload-pack --stateless-rpc --advertise-refs` and prepends the
//!   pkt-line service header to its output.
//! - `POST /{repo}.git/git-upload-pack` — pipes the request body (gunzipped
//!   when `Content-Encoding: gzip`) into `git upload-pack --stateless-rpc`
//!   stdin and streams stdout back as the response.
//! - receive-pack must never be exposed in any form: every
//!   `git-receive-pack` request is answered with 403 `read_only`. The
//!   rejection is wired up in `crate::routes`.
//!
//! The client's `Git-Protocol` header is forwarded as the `GIT_PROTOCOL`
//! environment variable so protocol v2 negotiation works. Only the resolved
//! git dir ever reaches the command line — the repository name from the URL
//! is validated by `repo::open::open_named` first.

use std::io::Read;
use std::path::Path as FsPath;
use std::process::Stdio;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::io::ReaderStream;
use utoipa::IntoParams;

use crate::error::{ApiError, ErrorResponse};
use crate::repo::open;
use crate::state::AppState;

const ADVERTISEMENT_CONTENT_TYPE: &str = "application/x-git-upload-pack-advertisement";
const RESULT_CONTENT_TYPE: &str = "application/x-git-upload-pack-result";

/// pkt-line service header + flush-pkt that precedes the advertisement over
/// HTTP (also under protocol v2, where the client skips `#` comment pkts).
const UPLOAD_PACK_PREAMBLE: &[u8] = b"001e# service=git-upload-pack\n0000";

/// Upper bound for the (possibly gzipped) upload-pack request body; the
/// negotiation data stays small even for huge repositories. Enforced by a
/// `DefaultBodyLimit` layer in `crate::routes` (413 when exceeded).
pub(crate) const MAX_REQUEST_BODY: usize = 8 * 1024 * 1024;

/// Upper bound for the request body after gzip inflation (zip-bomb defence).
const MAX_INFLATED_BODY: u64 = 64 * 1024 * 1024;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InfoRefsQuery {
    /// Must be `git-upload-pack`. `git-receive-pack` is `403 read_only`;
    /// anything else (including the dumb protocol's missing value) is
    /// `400 invalid_param`.
    #[param(example = "git-upload-pack")]
    service: Option<String>,
}

/// Ref advertisement (clone/fetch)
///
/// Spawns `git upload-pack --stateless-rpc --advertise-refs` and prepends the
/// pkt-line service header. A `Git-Protocol` request header is sanitized and
/// forwarded as `GIT_PROTOCOL`, so protocol v2 negotiation works.
#[utoipa::path(
    get,
    path = "/{repo_git}/info/refs",
    tag = "smart-http",
    params(
        ("repo_git" = String, Path, description = "Repository directory name **including** the `.git` suffix", example = "git-compose.git"),
        InfoRefsQuery,
    ),
    responses(
        (status = 200, description = "pkt-line service header followed by the ref advertisement",
            content_type = "application/x-git-upload-pack-advertisement",
            body = String,
            headers(("Cache-Control" = String, description = "Always `no-cache`; Smart HTTP never uses `ETag`")),
        ),
        (status = 400, description = "`invalid_param` — `service` missing or unsupported", body = ErrorResponse),
        (status = 403, description = "`read_only` — `service=git-receive-pack`", body = ErrorResponse),
        (status = 404, description = "`repo_not_found` — unknown repository, or a path without the `.git` suffix", body = ErrorResponse),
        (status = 500, description = "`internal` — upload-pack could not be spawned", body = ErrorResponse),
    ),
)]
pub async fn info_refs(
    State(state): State<AppState>,
    Path(repo_git): Path<String>,
    Query(query): Query<InfoRefsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    match query.service.as_deref() {
        Some("git-upload-pack") => {}
        // Push is SSH-only; the web surface stays read-only (CLAUDE.md
        // invariant), so receive-pack keeps its dedicated 403 contract.
        Some("git-receive-pack") => return Err(ApiError::ReadOnly),
        // The dumb protocol (no service parameter) is not supported.
        _ => {
            return Err(ApiError::InvalidParam(
                "smart HTTP requires service=git-upload-pack".to_owned(),
            ));
        }
    }

    let git_dir = resolve_git_dir(&state, &repo_git).await?;
    let output = upload_pack_command(&git_dir, true, git_protocol(&headers).as_deref())
        .output()
        .await
        .map_err(|err| {
            ApiError::Internal(anyhow::Error::new(err).context("failed to spawn git upload-pack"))
        })?;
    if !output.status.success() {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "git upload-pack --advertise-refs failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let body = [UPLOAD_PACK_PREAMBLE, &output.stdout].concat();
    Response::builder()
        .header(header::CONTENT_TYPE, ADVERTISEMENT_CONTENT_TYPE)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(body))
        .map_err(|err| ApiError::Internal(err.into()))
}

/// Upload-pack negotiation (clone/fetch)
///
/// Pipes the request body into `git upload-pack --stateless-rpc` and streams
/// its stdout back chunked. If upload-pack dies mid-stream the status code
/// cannot change, so the client sees an early EOF.
#[utoipa::path(
    post,
    path = "/{repo_git}/git-upload-pack",
    tag = "smart-http",
    params(
        ("repo_git" = String, Path, description = "Repository directory name **including** the `.git` suffix", example = "git-compose.git"),
    ),
    request_body(
        description = "upload-pack negotiation pkt-lines. `Content-Encoding: gzip` is inflated by the server. Limits: 8 MiB compressed, 64 MiB inflated.",
        content_type = "application/x-git-upload-pack-request",
        content = String,
    ),
    responses(
        (status = 200, description = "Packfile stream",
            content_type = "application/x-git-upload-pack-result",
            body = String,
            headers(("Cache-Control" = String, description = "Always `no-cache`; Smart HTTP never uses `ETag`")),
        ),
        (status = 400, description = "`invalid_param` — gzip inflation failed or the inflated body exceeded 64 MiB", body = ErrorResponse),
        (status = 404, description = "`repo_not_found`", body = ErrorResponse),
        (status = 413, description = "Compressed body over 8 MiB — axum's `DefaultBodyLimit` answers in plain text, not the JSON envelope"),
        (status = 500, description = "`internal` — upload-pack could not be spawned", body = ErrorResponse),
    ),
)]
pub async fn upload_pack(
    State(state): State<AppState>,
    Path(repo_git): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let git_dir = resolve_git_dir(&state, &repo_git).await?;
    let request = tokio::task::spawn_blocking(move || inflate_if_gzip(&headers, body))
        .await
        .map_err(|err| ApiError::Internal(err.into()))??;
    let protocol = request.protocol;

    let mut child = upload_pack_command(&git_dir, false, protocol.as_deref())
        .spawn()
        .map_err(|err| {
            ApiError::Internal(anyhow::Error::new(err).context("failed to spawn git upload-pack"))
        })?;

    let mut stdin = child.stdin.take().expect("stdin was configured as piped");
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let mut stderr = child.stderr.take().expect("stderr was configured as piped");

    // stateless-rpc reads the whole request before answering, but writing from
    // a separate task rules out any write/read deadlock. An EPIPE from a child
    // that died early is ignored here; the reaper below logs the real cause.
    tokio::spawn(async move {
        let _ = stdin.write_all(&request.body).await;
        // Dropping stdin closes the pipe and signals EOF to upload-pack.
    });

    // Reaper: collects stderr and waits so the child never becomes a zombie.
    // Failures past this point cannot change the status code (headers are
    // already sent); the stream just ends short and the client sees an early
    // EOF. A dropped Body closes the pipe and git exits on EPIPE.
    tokio::spawn(async move {
        let mut diagnostics = Vec::new();
        let _ = stderr.read_to_end(&mut diagnostics).await;
        match child.wait().await {
            Ok(status) if status.success() => {}
            outcome => tracing::error!(
                ?outcome,
                stderr = %String::from_utf8_lossy(&diagnostics),
                "git upload-pack exited abnormally"
            ),
        }
    });

    Response::builder()
        .header(header::CONTENT_TYPE, RESULT_CONTENT_TYPE)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(ReaderStream::new(stdout)))
        .map_err(|err| ApiError::Internal(err.into()))
}

/// Strips the mandatory `.git` suffix and resolves the repository's git dir.
/// Name validation (traversal rejection) is `open_named`'s, not duplicated.
async fn resolve_git_dir(state: &AppState, repo_git: &str) -> Result<std::path::PathBuf, ApiError> {
    let name = repo_name(repo_git)?.to_owned();
    let root = state.config.repo_root.clone();
    tokio::task::spawn_blocking(move || {
        let repo = open::open_named(&root, &name)?;
        Ok::<_, ApiError>(repo.path().to_path_buf())
    })
    .await
    .map_err(|err| ApiError::Internal(err.into()))?
}

fn repo_name(repo_git: &str) -> Result<&str, ApiError> {
    repo_git
        .strip_suffix(".git")
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ApiError::RepoNotFound(repo_git.to_owned()))
}

/// Returns the client's `Git-Protocol` header when it looks like a protocol
/// wish list (`version=2` etc.); anything unexpected is dropped rather than
/// exported into the child's environment.
fn git_protocol(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("git-protocol")?.to_str().ok()?;
    let valid = value.len() <= 64
        && !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'_' | b'-'));
    valid.then(|| value.to_owned())
}

struct UploadPackRequest {
    body: Vec<u8>,
    protocol: Option<String>,
}

/// Inflates the request body when `Content-Encoding: gzip` (git compresses
/// larger negotiation requests) and captures the protocol header while the
/// headers are still around. Blocking — call inside `spawn_blocking`.
fn inflate_if_gzip(headers: &HeaderMap, body: Bytes) -> Result<UploadPackRequest, ApiError> {
    let protocol = git_protocol(headers);
    let gzipped = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("gzip"));
    let body = if gzipped {
        let mut inflated = Vec::new();
        flate2::read::GzDecoder::new(body.as_ref())
            .take(MAX_INFLATED_BODY + 1)
            .read_to_end(&mut inflated)
            .map_err(|_| ApiError::InvalidParam("malformed gzip request body".to_owned()))?;
        if inflated.len() as u64 > MAX_INFLATED_BODY {
            return Err(ApiError::InvalidParam(
                "request body too large after inflation".to_owned(),
            ));
        }
        inflated
    } else {
        body.to_vec()
    };
    Ok(UploadPackRequest { body, protocol })
}

fn upload_pack_command(git_dir: &FsPath, advertise: bool, protocol: Option<&str>) -> Command {
    let mut command = Command::new("git");
    command.arg("upload-pack").arg("--stateless-rpc");
    if advertise {
        command.arg("--advertise-refs");
    }
    command
        .arg(git_dir)
        .stdin(if advertise {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(protocol) = protocol {
        command.env("GIT_PROTOCOL", protocol);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn preamble_length_matches_pkt_line_header() {
        // "001e" (hex 30) = 4 length bytes + the service line, then flush-pkt.
        let line_len = "# service=git-upload-pack\n".len() + 4;
        assert_eq!(line_len, 0x1e);
        assert!(UPLOAD_PACK_PREAMBLE.starts_with(b"001e"));
        assert!(UPLOAD_PACK_PREAMBLE.ends_with(b"0000"));
    }

    #[test]
    fn repo_name_should_require_git_suffix() {
        assert_eq!(repo_name("alpha.git").unwrap(), "alpha");
        for invalid in ["alpha", ".git", "alpha.GIT", ""] {
            assert!(
                matches!(repo_name(invalid), Err(ApiError::RepoNotFound(_))),
                "{invalid:?} was not rejected"
            );
        }
    }

    #[test]
    fn git_protocol_should_pass_only_sane_values() {
        let mut headers = HeaderMap::new();
        assert_eq!(git_protocol(&headers), None);

        headers.insert("git-protocol", "version=2".parse().unwrap());
        assert_eq!(git_protocol(&headers), Some("version=2".to_owned()));

        for bad in ["version 2", "a".repeat(65).as_str(), "", "x;y"] {
            headers.insert("git-protocol", bad.parse().unwrap());
            assert_eq!(git_protocol(&headers), None, "{bad:?} was not dropped");
        }
    }

    #[test]
    fn inflate_if_gzip_should_pass_plain_bodies_through() {
        let request =
            inflate_if_gzip(&HeaderMap::new(), Bytes::from_static(b"0000")).expect("plain body");
        assert_eq!(request.body, b"0000");
        assert_eq!(request.protocol, None);
    }

    #[test]
    fn inflate_if_gzip_should_inflate_gzip_bodies() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"0009done\n").unwrap();
        let gzipped = encoder.finish().unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        let request = inflate_if_gzip(&headers, Bytes::from(gzipped)).expect("gzip body");
        assert_eq!(request.body, b"0009done\n");
    }

    #[test]
    fn inflate_if_gzip_should_reject_malformed_gzip() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        let result = inflate_if_gzip(&headers, Bytes::from_static(b"not gzip"));
        assert!(matches!(result, Err(ApiError::InvalidParam(_))));
    }
}
