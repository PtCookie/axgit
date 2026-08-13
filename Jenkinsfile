// Image build (Dockerfile) is wired in separately later. Assumes the agent already has
// Node.js and a Rust toolchain (rustc/cargo, rustfmt + clippy components) installed; this
// file only pins the exact pnpm version via corepack. The Release stage (v* tag builds
// only, docs/DECISIONS.md #75) additionally needs musl-tools and the
// x86_64-unknown-linux-musl rustup target for the single-binary release tarball
// (scripts/make-release.sh).
pipeline {
    agent any

    options {
        buildDiscarder(logRotator(numToKeepStr: '20'))
        timestamps()
        timeout(time: 30, unit: 'MINUTES')
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

        // Single-binary release tarball (scripts/make-release.sh, docs/DECISIONS.md #75):
        // only on a v* tag build, after Test has passed. Target is fixed to
        // x86_64-unknown-linux-musl, matching the Dockerfile's static-linking posture
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
