SHELL := /bin/bash
.SHELLFLAGS := -euo pipefail -c

.PHONY: build check test acceptance provider-test

provider-test:
	node --test assets/providers/cursor.test.mjs

build:
	cargo build --locked -p mas-host

check:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	@test -z "$$(gofmt -l sdk/go examples/go-release)" || { echo 'Run gofmt on Go source.' >&2; exit 1; }
	cd sdk/go && go vet ./...
	cd examples/go-release && go vet ./...

test: check
	cargo test --locked --workspace
	cd sdk/go && go test -race ./...
	cd examples/go-release && go test -race ./...
	mkdir -p target
	c++ -std=c++17 -Wall -Wextra -Werror -pthread -Isdk/cpp/include sdk/cpp/tests/conformance.cpp -o target/cpp-conformance
	target/cpp-conformance

acceptance:
	cargo test --locked -p mas-host --test acceptance -- --ignored --nocapture
