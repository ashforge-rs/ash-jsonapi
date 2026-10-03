.PHONY: check build fmt lint doc doc-test features pre-commit bench \
        test test-bare deny package dry-run publish

check:
	cargo check --all-features

build:
	cargo build --all-features

fmt:
	cargo fmt --all

lint:
	cargo clippy --all-features --all-targets -- -D warnings

doc:
	cargo doc --all-features --no-deps

# The feature matrix is the thing most likely to break silently, so this
# runs locally as part of pre-commit as well as in CI.
features:
	cargo check --no-default-features
	@for f in http audit domain validation streaming; do \
		echo "--- $$f ---"; \
		cargo check --no-default-features --features $$f || exit 1; \
	done
	cargo check --all-features

# Where a response spends its time. Not part of `pre-commit`: a benchmark is
# a measurement, not a gate — it is noisy on a loaded machine and would fail
# a commit for reasons that have nothing to do with the commit.
#
# Criterion keeps the previous run in `target/criterion`, so a second
# invocation prints the change against it. Measure, edit, measure again.
bench:
	cargo bench --bench serialize

# The crate-level example is a real doc-test, because it is the first thing
# anyone reads.
doc-test:
	cargo test --doc

# The bare build's *tests*, not just its `check`.
#
# `features` above compiles the library under each feature set, which is what
# caught feature-gating mistakes in `src/`. It never built the test targets,
# so every test silently assumed `http` + `validation` and
# `cargo test --no-default-features` failed to compile for a long time without
# anyone noticing. This is the step that keeps the standalone claim — the
# document, error, query and serialization types with no `ash-*` crate in the
# tree — actually exercised rather than merely asserted.
#
# `--tests` and not a plain `cargo test`: doc-tests describe the
# default-featured crate (the front-page example builds a whole axum service),
# and a doc example cannot carry `required-features` the way a `[[test]]`
# target can. They are covered by `doc-test` above.
test-bare:
	cargo test --no-default-features --tests

test:
	cargo test --all-features

pre-commit: fmt lint check features test test-bare doc-test

# ---------------------------------------------------------------------------
# Release
# ---------------------------------------------------------------------------

# Advisories, licences, bans and sources. This crate's dependency tree becomes
# its consumers', so an advisory here propagates rather than stopping with us.
# Needs `cargo install cargo-deny`.
deny:
	cargo deny check advisories licenses bans sources

# What the tarball would actually contain. Worth reading before a first
# publish: `exclude` in Cargo.toml is easy to get subtly wrong, and the
# published set is not something a later version can take back.
package:
	cargo package --list

dry-run:
	cargo publish --dry-run --all-features

publish:
	cargo publish
