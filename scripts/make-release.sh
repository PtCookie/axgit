#!/usr/bin/env bash
# Packages release tarballs for the single-binary, non-container deploy path
# (the `embed-web` Cargo feature, docs/DECISIONS.md #74/#75/#79): builds `axgit`
# with `web/dist` baked in for each target in $TARGETS, then stages each one
# together with the systemd unit, env-file template, and install docs into
#   release/axgit-$VERSION-$TARGET.tar.gz  (one per target)
#   release/SHA256SUMS                     (covers every tarball produced)
#
# Assumes `pnpm install` has already run (Jenkins' "Install dependencies"
# stage does this) — this script does not install JS/Rust dependencies
# itself, only builds and packages.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_DIR="$ROOT/release"

# ---------------------------------------------------------------------------
# Helpers (used by both toolchain resolution and the smoke check below).
# ---------------------------------------------------------------------------

target_os() {
  case "$1" in
  *-linux-*) echo linux ;;
  *-apple-darwin) echo darwin ;;
  *) echo unknown ;;
  esac
}
target_arch() {
  case "$1" in
  x86_64-*) echo x86_64 ;;
  aarch64-*) echo aarch64 ;;
  arm64-*) echo aarch64 ;;
  *) echo unknown ;;
  esac
}

# True if $1 can run directly on this host (same OS + arch as $HOST_TRIPLE) —
# the same predicate the smoke check uses to decide whether a runner is
# needed. NOTE: arch alone is not enough — e.g. macOS/aarch64 building
# aarch64-unknown-linux-musl matches on arch but is unambiguously a cross
# build (different OS), and must not be treated as native.
is_native() {
  [[ "$(target_os "$HOST_TRIPLE")" == "$(target_os "$1")" \
    && "$(target_arch "$HOST_TRIPLE")" == "$(target_arch "$1")" \
    && "$(target_os "$1")" != unknown ]]
}

# Runs `$@ $binary --version` and checks it printed this release's version.
# The binary has never actually been executed up to this point — a musl
# cc/linker misconfiguration can still produce a file that exists, passes
# -x, and immediately segfaults or fails to start. Catches that plus a
# tag/binary version mismatch before anything is packaged.
run_smoke_check() {
  local binary="$1" expected_version="$2"
  shift 2
  local actual_version
  actual_version="$("$@" "$binary" --version)"
  if [[ "$actual_version" != "axgit $expected_version" ]]; then
    echo "error: '$* $binary --version' printed '$actual_version', expected 'axgit $expected_version'" >&2
    exit 1
  fi
}

# Decides how (or whether) to run $binary (built for $t) on this host, then
# delegates the actual check to run_smoke_check. Deliberately does NOT fall
# back to "just try executing it" for a non-native target: a genuinely
# broken binary also fails to execute, and a blanket try/skip would turn
# that hard failure into a silent pass — exactly the failure mode this
# check exists to catch (docs/DECISIONS.md #76).
smoke_check() {
  local t="$1" binary="$2"
  local arch runner_var runner binfmt_file

  arch="$(target_arch "$t")"
  runner_var="CARGO_TARGET_$(echo "$t" | tr 'a-z-' 'A-Z_')_RUNNER"
  runner="${!runner_var:-}"

  if [[ -n "$runner" ]]; then
    echo "==> Smoke-checking $t via \$$runner_var"
    run_smoke_check "$binary" "$VERSION" $runner
    return
  fi

  if is_native "$t"; then
    echo "==> Smoke-checking the built binary (--version)"
    run_smoke_check "$binary" "$VERSION"
    return
  fi

  if [[ "$(target_os "$HOST_TRIPLE")" == linux && "$(target_os "$t")" == linux ]]; then
    # No -L <sysroot> is needed for either qemu invocation below: the
    # release binary is a statically linked musl executable, which is
    # exactly the case naive qemu-user usage otherwise trips on.
    if command -v "qemu-${arch}-static" >/dev/null 2>&1; then
      echo "==> Smoke-checking $t via qemu-${arch}-static"
      run_smoke_check "$binary" "$VERSION" "qemu-${arch}-static"
      return
    fi
    if command -v "qemu-${arch}" >/dev/null 2>&1; then
      echo "==> Smoke-checking $t via qemu-${arch}"
      run_smoke_check "$binary" "$VERSION" "qemu-${arch}"
      return
    fi
    if [[ -r /proc/sys/fs/binfmt_misc/status ]] &&
      grep -qx enabled /proc/sys/fs/binfmt_misc/status; then
      for binfmt_file in /proc/sys/fs/binfmt_misc/*"${arch}"*; do
        if [[ -r "$binfmt_file" ]] && grep -qx enabled "$binfmt_file"; then
          echo "==> Smoke-checking $t via binfmt_misc (direct exec)"
          run_smoke_check "$binary" "$VERSION"
          return
        fi
      done
    fi
  fi

  echo "warning: skipping smoke check for $t — host ($HOST_TRIPLE) cannot run it and no" >&2
  echo "  runner was found (qemu-${arch}-static / qemu-${arch} on PATH, or a registered" >&2
  echo "  binfmt_misc handler). Install the 'qemu-user-static' package, or set" >&2
  echo "  \$$runner_var, to verify this binary before release — otherwise it ships unexecuted." >&2
}

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

HOST_TRIPLE="$(rustc -vV | awk '/^host: /{ print $2 }')"

# Space-separated, not an array: keeps `TARGET=aarch64-apple-darwin
# ./scripts/make-release.sh` (the documented non-Linux local-verification
# path) working unchanged as a single-target alias, and avoids bash 3.2's
# "unbound variable" on an empty array under `set -u`.
TARGETS="${TARGETS:-${TARGET:-x86_64-unknown-linux-musl aarch64-unknown-linux-musl}}"

VERSION="$(awk -F'"' '/^version = /{ print $2; exit }' "$ROOT/api/Cargo.toml")"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [package] version from api/Cargo.toml" >&2
  exit 1
fi

# Jenkins sets TAG_NAME on a tag build (release trigger, docs/DECISIONS.md
# #75). A mismatch between the pushed tag and the crate version should stop
# the release rather than ship a mislabelled tarball.
if [[ -n "${TAG_NAME:-}" && "$TAG_NAME" != "v$VERSION" ]]; then
  echo "error: TAG_NAME '$TAG_NAME' does not match api/Cargo.toml version 'v$VERSION'" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Pass 1: validate every target and resolve its toolchain, before building
# anything. Doing this up front means a missing aarch64 cross compiler is
# caught immediately, not after the x86_64 leg has already finished building.
# ---------------------------------------------------------------------------

echo "==> Resolving toolchains for: $TARGETS"
for t in $TARGETS; do
  if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed | grep -qx "$t"; then
      echo "error: rust target '$t' is not installed. Install it first:" >&2
      echo "  rustup target add $t" >&2
      exit 1
    fi
  fi

  # Only *-musl targets need a target-specific C compiler: libgit2-sys,
  # libz-sys, liblzma-sys, and zstd-sys all compile vendored C sources via
  # the `cc` crate, which otherwise reaches for the host's glibc-targeting
  # compiler. The linker itself is left at rustc's default (self-contained
  # musl crt) — except on a genuine cross build, see below.
  case "$t" in
  *-musl) ;;
  *) continue ;;
  esac

  t_u="$(echo "$t" | tr '-' '_')"
  t_U="$(echo "$t" | tr 'a-z-' 'A-Z_')"
  arch="$(target_arch "$t")"

  if is_native "$t"; then
    mode=native
  else
    mode=cross
  fi

  # A pre-set CC_<triple> in the environment IS the override mechanism —
  # cc-rs's own lookup order is CC_<triple-dashes> > CC_<triple_underscores>
  # > TARGET_CC > CC, so exporting our own CC_<triple_underscores> below
  # would silently shadow a TARGET_CC an operator set instead. Honour
  # whatever is already there rather than introducing a second variable.
  cc_var="CC_${t_u}"
  cc="${!cc_var:-}"

  if [[ -z "$cc" ]]; then
    if [[ "$mode" == native ]]; then
      # musl-gcc (musl-tools) is tried first since it's what a native musl
      # host/agent already has today; the two cross-style names are tried
      # too since they also work natively where installed.
      candidates="musl-gcc ${arch}-linux-musl-gcc ${t}-gcc"
    else
      # Two cross-toolchain naming conventions: musl.cc / Homebrew
      # musl-cross (<arch>-linux-musl-gcc — the one cc-rs auto-derives for
      # aarch64 without any env var) and messense/macos-cross-toolchains
      # (<triple>-gcc — cc-rs does NOT know this one, so it only works via
      # the explicit CC_<triple_underscores> export below).
      candidates="${arch}-linux-musl-gcc ${t}-gcc"
    fi
    cc=""
    for candidate in $candidates; do
      if command -v "$candidate" >/dev/null 2>&1; then
        cc="$candidate"
        break
      fi
    done
    if [[ -z "$cc" ]]; then
      echo "error: no musl C compiler found for target '$t'." >&2
      echo "  Tried: $candidates" >&2
      echo "  Install musl-tools (native) or a cross toolchain matching one of the names" >&2
      echo "  above (e.g. 'brew install messense/macos-cross-toolchains/$t' on macOS, or" >&2
      echo "  musl.cc's ${arch}-linux-musl-cross on Linux), or set ${cc_var}=/path/to/gcc." >&2
      exit 1
    fi
    export "${cc_var}=${cc}"
    echo "==> $t: CC=$cc"
  else
    echo "==> $t: CC=$cc (from \$$cc_var)"
  fi

  if [[ "$mode" == cross ]]; then
    # REQUIRED for a cross build: rustc's musl target spec has no "linker"
    # key and linker-flavor "gnu-cc", so rustc drives the link through
    # PATH's `cc` regardless of self-contained crt/libc.a — which cannot
    # link foreign-arch objects. Deliberately left unset for a native
    # target to preserve today's already-proven x86_64-on-x86_64 behaviour.
    linker_var="CARGO_TARGET_${t_U}_LINKER"
    if [[ -z "${!linker_var:-}" ]]; then
      export "${linker_var}=${cc}"
    fi
    # Best-effort: cc-rs auto-derives <prefix>-ar only under the musl.cc
    # naming convention and otherwise falls back to the host's bare `ar` —
    # which cannot index an ELF archive on a macOS host. Not exporting
    # RANLIB: cc 1.4.0 never invokes ranlib internally, so it would be dead.
    ar_var="AR_${t_u}"
    ar_candidate="${cc%-gcc}-ar"
    if [[ -z "${!ar_var:-}" ]] && command -v "$ar_candidate" >/dev/null 2>&1; then
      export "${ar_var}=${ar_candidate}"
    fi
  fi

  # Do NOT set PKG_CONFIG_ALLOW_CROSS, PKG_CONFIG, or PKG_CONFIG_SYSROOT_DIR
  # here or in the environment this script runs in. `git2` doesn't enable
  # libgit2-sys's `vendored` feature, so libgit2-sys always probes for a
  # system libgit2 first; on a cross build that probe is refused by
  # pkg-config itself (host != target) and only *that* refusal is what
  # forces the vendored-C fallback this whole build relies on. Any of the
  # three variables above defeats the refusal and lets libgit2-sys/libz-sys
  # link against the host's glibc .pc files instead — producing a binary
  # that looks statically built but silently isn't (docs/DECISIONS.md #79).
done

echo "==> Building frontend (pnpm --filter web build)"
(cd "$ROOT" && pnpm --filter web build)

rm -rf "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR"

# ---------------------------------------------------------------------------
# Pass 2: build, smoke-check, and package each target.
# ---------------------------------------------------------------------------

TARBALLS=""

for t in $TARGETS; do
  echo "==> Building axgit $VERSION for $t (--features embed-web)"
  (cd "$ROOT" && cargo build --release --locked \
    --manifest-path api/Cargo.toml \
    --features embed-web \
    --target "$t")

  BINARY="$ROOT/api/target/$t/release/axgit"
  if [[ ! -x "$BINARY" ]]; then
    echo "error: expected binary not found at $BINARY" >&2
    exit 1
  fi

  smoke_check "$t" "$BINARY"

  PKG_NAME="axgit-$VERSION-$t"
  STAGE_DIR="$RELEASE_DIR/$PKG_NAME"

  echo "==> Staging $PKG_NAME"
  rm -rf "$STAGE_DIR"
  mkdir -p "$STAGE_DIR"
  cp "$BINARY" "$STAGE_DIR/axgit"
  cp "$ROOT/packaging/axgit.service" "$STAGE_DIR/"
  cp "$ROOT/packaging/axgit.env.example" "$STAGE_DIR/"
  cp "$ROOT/packaging/INSTALL.md" "$STAGE_DIR/"
  cp "$ROOT/LICENSE" "$STAGE_DIR/"

  TARBALL="$RELEASE_DIR/$PKG_NAME.tar.gz"
  echo "==> Writing $TARBALL"
  if tar --version 2>/dev/null | grep -q GNU; then
    # Reproducible archive: fixed ownership/order/mtime regardless of build host.
    tar -C "$RELEASE_DIR" \
      --sort=name --mtime='UTC 2026-01-01' --owner=0 --group=0 --numeric-owner \
      -czf "$TARBALL" "$PKG_NAME"
  else
    # bsdtar (macOS) has no --sort/--mtime/--numeric-owner; used for local
    # verification only, not the Jenkins release artifact.
    tar -C "$RELEASE_DIR" -czf "$TARBALL" "$PKG_NAME"
  fi
  rm -rf "$STAGE_DIR"

  TARBALLS="$TARBALLS $PKG_NAME.tar.gz"
done

echo "==> Writing checksums"
SUMS="$RELEASE_DIR/SHA256SUMS"
(
  cd "$RELEASE_DIR"
  # From the recorded list, never a glob — so a leftover file from outside
  # this run can never end up checksummed.
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum $TARBALLS >"$SUMS"
  else
    shasum -a 256 $TARBALLS >"$SUMS"
  fi
)

echo "==> Done"
for tb in $TARBALLS; do
  echo "  $RELEASE_DIR/$tb"
done
echo "  $SUMS"
