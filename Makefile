.PHONY: build fmt fmt-check lint test doctest check deny audit machete supply-chain dist-plan

build:
	cargo build --release --locked

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

lint:
	cargo clippy --locked --all-targets -- -D warnings

test:
	cargo nextest run --locked

doctest:
	cargo test --locked --doc

check: fmt-check lint test doctest

deny:
	cargo deny check

audit:
	cargo audit

machete:
	cargo machete

supply-chain: deny audit machete

dist-plan:
	dist plan
