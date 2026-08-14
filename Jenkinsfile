// Image build (Containerfile) is wired in separately later. Assumes the agent already has
// Node.js and a Rust toolchain (rustc/cargo, rustfmt + clippy components) installed; this
// file only pins the exact pnpm version via corepack. The Release stage (v* tag builds
// only, docs/DECISIONS.md #75/#79) builds BOTH x86_64-unknown-linux-musl and
// aarch64-unknown-linux-musl (scripts/make-release.sh) and additionally needs, on the
// agent: both rustup targets; musl-tools (musl-gcc, for the native x86_64 leg); an
// aarch64 cross musl gcc for the aarch64 leg (musl.cc's aarch64-linux-musl-gcc or
// messense/macos-cross-toolchains' aarch64-unknown-linux-musl-gcc); and, recommended,
// the qemu-user-static package so the aarch64 binary's smoke check actually executes
// instead of shipping unverified. The Embedded build stage (every build,
// docs/DECISIONS.md #76) builds `embed-web` for the host target only, so it needs none
// of the above.
pipeline {
    agent any

    options {
        buildDiscarder(logRotator(numToKeepStr: '20'))
        timestamps()
        // Raised from 30 to 45 minutes for docs/DECISIONS.md #79: a v* tag build now pays
        // two full `--release` builds in the Release stage (x86_64 + aarch64, each a
        // fresh vendored libgit2/xz/zstd/zlib C build) on top of the parallel Test stage
        // and the Embedded build stage, all sharing this one pipeline-wide timeout.
        timeout(time: 45, unit: 'MINUTES')
    }

    stages {
        stage('Install dependencies') {
            steps {
                sh 'pnpm install --frozen-lockfile'
                sh 'pnpm --filter web exec playwright install'
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
                                sh 'pnpm --filter web test -- --run'
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

        // The code the release actually ships (docs/DECISIONS.md #74/#76): `embed-web` is off
        // in the Test stage above, so assets.rs's embedded arms, routes.rs's embedded
        // fallback, and tests/embedded_assets_test.rs/static_shell_test.rs's embed variants
        // are compiled out on every ordinary build — without this stage, a v* tag's Release
        // stage would be the first thing to ever compile them. This is also the only place
        // CI runs the production `astro build` (Playwright's webServer uses `pnpm dev`
        // instead), which the feature needs regardless: rust-embed reads web/dist at compile
        // time. Runs on every build, not just tags, so a break surfaces immediately instead
        // of at release time.
        stage('Embedded build') {
            steps {
                sh 'pnpm --filter web build'
                sh 'cargo clippy --manifest-path api/Cargo.toml --features embed-web --all-targets -- -D warnings'
                // Scoped to the embed-specific targets rather than the full integration
                // suite: clippy --all-targets above already proves everything compiles under
                // the feature, and re-running every other test unchanged by embed-web would
                // just spend the 30-minute pipeline timeout on the parallel Test stage's work
                // a second time.
                sh 'cargo test --manifest-path api/Cargo.toml --features embed-web --lib --test embedded_assets_test --test static_shell_test'
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
