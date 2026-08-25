.PHONY: lint format fmt-check check test e2e ci

lint:
	cargo clippy --workspace --all-targets -- -D warnings

format:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check: lint format

test:
	cargo test --workspace --all-targets

e2e:
	bash tests/e2e/local.sh

ci: fmt-check lint test
