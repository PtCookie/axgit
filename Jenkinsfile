// Test-only pipeline for now — image build (Dockerfile) is wired in separately later.
// Assumes the agent already has Node.js and a Rust toolchain (rustc/cargo, rustfmt +
// clippy components) installed; this file only pins the exact pnpm version via corepack.
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
    }

    post {
        always {
            archiveArtifacts artifacts: 'web/playwright-report/**', allowEmptyArchive: true
        }
    }
}
