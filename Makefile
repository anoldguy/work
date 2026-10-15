.PHONY: build fmt fmt-check lint test doctest check deny audit machete supply-chain release

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

# `make release v1.2.3`: the version is the second goal, so it gets a no-op rule.
ifeq (release,$(firstword $(MAKECMDGOALS)))
RELEASE_VERSION := $(word 2,$(MAKECMDGOALS))
$(eval $(RELEASE_VERSION):;@:)
endif

release:
	@$(if $(shell command -v mise),mise exec --) scripts/release.sh $(RELEASE_VERSION)
