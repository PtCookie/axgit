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
                //sh 'corepack enable'
                //sh 'corepack prepare pnpm@11.17.0 --activate'
                sh 'pnpm install --frozen-lockfile'
                // Browser binaries aren't part of the npm packages; both vitest's
                // browser mode and the Playwright e2e suite need a real Chromium.
                // Drop --with-deps if the agent's user can't run apt/sudo.
                sh 'pnpm --filter web exec playwright install --with-deps chromium'
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
                                sh 'cargo test --manifest-path api/Cargo.toml'
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
