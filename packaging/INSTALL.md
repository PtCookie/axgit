# Installing axgit (single-binary / systemd)

This tarball is a complete deployment on its own: the `axgit` binary has the frontend baked in
(the `embed-web` Cargo feature, docs/DECISIONS.md #74), so nothing else from the build needs to be
copied. A `git` binary must still be on `PATH` — it's used for archive downloads and Smart HTTP
clone/fetch (docs/ARCHITECTURE.md's hybrid libgit2+exec policy).

This is an alternative to the container image (see the repo root README.md) for a bare-metal or
VM install managed by systemd.

Releases ship one tarball per CPU architecture (docs/DECISIONS.md #79); the tarball's filename is
the only place that's recorded, so before installing, make sure you downloaded the one matching
the target machine — check with `uname -m` (`x86_64` or `aarch64`; on Linux, `aarch64` is arm64)
and match it against the `axgit-<version>-<target>.tar.gz` name's target triple.

## 1. Install the binary

```sh
sudo install -m 755 axgit /usr/local/bin/axgit
```

## 2. Choose the service user

Run the service as **the user that owns the repository root** (`AXGIT_REPO_ROOT`), not as a
dedicated unprivileged user with no relation to the repositories. This matters because axgit reads
bare repositories directly with libgit2 and shells out to `git`, both of which refuse to operate
on a directory owned by a different uid ("dubious ownership") unless a `safe.directory` config
entry is added. When the process uid already matches the repositories' owning uid, that check
passes outright and no `safe.directory` entry is needed anywhere — this was verified by reading
libgit2's ownership-check source directly (docs/DECISIONS.md #74).

If your repositories are owned by e.g. a `git` user, run axgit as that same user.

## 3. Install the unit and config

```sh
sudo cp axgit.service /etc/systemd/system/axgit.service
sudo mkdir -p /etc/axgit
sudo cp axgit.env.example /etc/axgit/axgit.env
sudo "$EDITOR" /etc/systemd/system/axgit.service   # set User=/Group= to the repo-owning account
sudo "$EDITOR" /etc/axgit/axgit.env                # set AXGIT_REPO_ROOT, AXGIT_CLONE_URL_BASE, etc.
```

Every setting is documented in `axgit.env.example` and in the repo root README's configuration
table — the two lists match.

## 4. Start it

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now axgit
sudo systemctl status axgit
```

axgit listens on `AXGIT_LISTEN` (default `0.0.0.0:8080`) and has no built-in TLS — put a reverse
proxy in front of it for a public deployment (docs/DECISIONS.md #10), the same assumption
`docs/compose.example.yaml` makes for the container deployment.

## 5. Verify

```sh
curl -s http://127.0.0.1:8080/api/v1/repos
```

should return a JSON array (empty if `AXGIT_REPO_ROOT` has no repositories yet), and
`http://127.0.0.1:8080/` should serve the web UI — with no `AXGIT_STATIC_DIR` set, this is the
frontend baked into the binary.

## Upgrading

```sh
sudo systemctl stop axgit
sudo install -m 755 axgit /usr/local/bin/axgit   # from a newer release tarball
sudo systemctl start axgit
```

Repository data is never touched by axgit itself (it's strictly read-only), so an upgrade is just
a binary swap.
