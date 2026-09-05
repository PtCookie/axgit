// Image build (Containerfile) is wired in separately later. Assumes the agent already has
// Node.js and a Rust toolchain (rustc/cargo, rustfmt + clippy components) installed; this
// file only pins the exact pnpm version via corepack. The Release stage (v* tag builds
// only, docs/DECISIONS.md #75/#79) builds BOTH x86_64-unknown-linux-musl and
// aarch64-unknown-linux-musl (scripts/make-release.sh) and additionally needs, on the
// agent: both rustup targets; musl-tools (musl-gcc, for the native x86_64 leg); an
// aarch64 cross musl gcc for the aarch64 leg (musl.cc's aarch64-linux-musl-gcc or
// messense/macos-cross-toolchains' aarch64-unknown-linux-musl-gcc); and, recommended,
// the qemu-user-static package so the aarch64 binary's smoke check actually executes
// instead of shipping unverified.
//
// The default cargo build bakes `web/dist` into the binary (docs/DECISIONS.md #74, #88),
// so `Build web` below runs before the `api` branch of the Test stage can compile at
// all — unlike the old `embed-web` opt-in feature, there's no default build left that
// skips the frontend. The `api-only` Cargo feature is the now-exceptional opt-out (a
// pure-API build with no bundled frontend); the `api` branch checks it too, scoped to the
// bits it actually changes rather than the full integration suite a second time.
pipeline {
    agent any

    options {
        buildDiscarder(logRotator(numToKeepStr: '20'))
        timestamps()
        // Raised from 30 to 45 minutes for docs/DECISIONS.md #79: a v* tag build now pays
        // two full `--release` builds in the Release stage (x86_64 + aarch64, each a
        // fresh vendored libgit2/xz/zstd/zlib C build) on top of the Build web and Test
        // stages, all sharing this one pipeline-wide timeout.
        timeout(time: 45, unit: 'MINUTES')
    }

    stages {
        stage('Install dependencies') {
            steps {
                sh 'pnpm install --frozen-lockfile'
                sh 'pnpm --filter web exec playwright install'
            }
        }

        // Ahead of the Test stage, not inside its `api` branch: the default cargo build
        // bakes this output into the binary (docs/DECISIONS.md #74, #88), so every `cargo`
        // invocation below — including the `api-only` opt-out check, which still needs
        // `web/dist` to exist for `embedded_assets_test.rs`'s counterpart on the default
        // build to compile in the same `cargo test` invocation — depends on it existing
        // first. This is also the only place CI runs the production `astro build`
        // (Playwright's webServer under the `web` branch uses `pnpm dev` instead).
        stage('Build web') {
            steps {
                sh 'pnpm --filter web build'
            }
        }

        stage('Test') {
            parallel {
                stage('api') {
                    stages {
                        stage('rustfmt') {
                            steps {
                                sh 'cargo fmt --manifest-path api/Cargo.toml -- --check'
                            }
                        }
                        stage('clippy') {
                            steps {
                                sh 'cargo clippy --manifest-path api/Cargo.toml --all-targets -- -D warnings'
                            }
                        }
                        stage('cargo test') {
                            steps {
                                // --all-targets skips doctests (there are none in this crate); the
                                // doctest step links against the system libgit2 in its own rustdoc
                                // invocation and has been flaky on the Jenkins agent.
                                sh 'cargo test --manifest-path api/Cargo.toml --all-targets'
                            }
                        }
                        // The `api-only` feature (docs/DECISIONS.md #88) is the opt-out from
                        // the default embedded build — a pure-API binary with no bundled
                        // frontend and no `web/dist` compile-time dependency. Checked here,
                        // scoped to what the feature actually changes, rather than as a
                        // separate pipeline stage: clippy --all-targets proves it still
                        // compiles end to end, and re-running every other test unchanged by
                        // the feature would just spend the pipeline timeout twice.
                        stage('api-only clippy') {
                            steps {
                                sh 'cargo clippy --manifest-path api/Cargo.toml --features api-only --all-targets -- -D warnings'
                            }
                        }
                        stage('api-only test') {
                            steps {
                                sh 'cargo test --manifest-path api/Cargo.toml --features api-only --lib --test static_shell_test'
                            }
                        }
                    }
                }

                stage('web') {
                    stages {
                        stage('lint & format') {
                            steps {
                                sh 'pnpm --filter web check'
                            }
                        }
                        stage('vitest') {
                            steps {
                                sh 'pnpm --filter web test --run'
                            }
                        }
                        stage('playwright e2e') {
                            steps {
                                sh 'pnpm --filter web test:e2e'
                            }
                        }
                    }
                }
            }
        }

        // Single-binary release tarballs (scripts/make-release.sh, docs/DECISIONS.md
        // #75/#79): only on a v* tag build, after Test has passed. Builds both
        // x86_64-unknown-linux-musl and aarch64-unknown-linux-musl (the script's
        // $TARGETS default), matching the Containerfile's static-linking posture
        // (docs/DECISIONS.md #22) without depending on Docker in this pipeline.
        stage('Release') {
            when {
                allOf {
                    buildingTag()
                    tag pattern: 'v.*', comparator: 'REGEXP'
                }
            }
            steps {
                sh './scripts/make-release.sh'
                archiveArtifacts artifacts: 'release/*.tar.gz,release/SHA256SUMS', fingerprint: true
            }
        }
    }

    post {
        always {
            archiveArtifacts artifacts: 'web/playwright-report/**', allowEmptyArchive: true
        }
    }
}
