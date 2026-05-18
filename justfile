## slint-mapping recipes
##
## Run `just` (no args) to list everything. Most recipes wrap one or
## two `cargo` invocations; the value-add over typing cargo directly
## is documentation + sensible default flags + the `publish-check`
## aggregator.

_default:
    @just --list --unsorted

## ─── Development loop ──────────────────────────────────────────────

# Format every Rust source in-place.
fmt:
    cargo fmt --all

# Refuse to modify; useful in CI.
fmt-check:
    cargo fmt --all -- --check

# Lint everything, denying warnings.
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# Type-check the whole workspace (compiles every .slint via build.rs).
check:
    cargo check --all-targets --all-features

# Run unit + integration tests.
test:
    cargo test --all-features

# What CI runs on every PR.
ci: fmt-check clippy check test

## ─── Pre-publish hygiene tools ─────────────────────────────────────

# Install every tool the publish-check recipes depend on (idempotent).
install-publish-tools:
    cargo install --locked cargo-semver-checks
    cargo install --locked cargo-audit
    cargo install --locked cargo-deny
    cargo install --locked cargo-machete
    cargo install --locked cargo-msrv
    cargo install --locked cargo-public-api
    cargo install --locked cargo-release
    cargo install --locked typos-cli

# Detect breaking API changes since the last released version.
semver-check:
    cargo semver-checks check-release

# Scan deps for known CVEs (RustSec advisory database).
audit:
    cargo audit

# Enforce license / advisory / source policy (config: deny.toml).
deny:
    cargo deny check

# Find dependencies listed in Cargo.toml but never used.
machete:
    cargo machete

# Bisect the minimum Rust toolchain that still compiles the crate.
msrv:
    cargo msrv find

# Diff the public API against the last published version.
public-api:
    cargo public-api --diff-git-checkouts

# Build docs the way docs.rs builds them (--no-deps --all-features).
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# Look for typos in source, comments, and docs.
typos:
    typos

# Run every pre-publish check (~minutes); fast-fail tools run first.
publish-check: fmt-check typos clippy check test doc machete audit deny semver-check
    @echo "──────────────────────────────────────────"
    @echo "  publish-check ✓ — slint-mapping is shippable"
    @echo "──────────────────────────────────────────"

## ─── Release flow ──────────────────────────────────────────────────

# Dry-run the publish step without uploading.
publish-dry:
    cargo publish --dry-run

# Bump version + tag + push + publish via cargo-release (run publish-check first).
# Examples: `just release patch` (0.1.0 → 0.1.1), `just release minor`, `just release 0.3.0`.
release LEVEL="patch":
    cargo release {{LEVEL}} --execute
