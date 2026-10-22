SHELL := /bin/bash
.SHELLFLAGS := -euo pipefail -c

.PHONY: build check test acceptance provider-test python-test workload-test workload-acceptance privacy-test notification-test capability-test governance-test

capability-test:
	npm --prefix examples/structured-evidence/typescript ci --ignore-scripts
	npm --prefix examples/structured-evidence/typescript run check
	npm --prefix examples/structured-evidence/typescript run build
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s examples/structured-evidence/tests -v

python-test:
	PYTHONPATH=sdk/python python3 -m unittest discover -s sdk/python/tests -v

workload-test:
	python3 -m unittest discover -s tests/performance -p 'test_*.py' -v

workload-acceptance: build workload-test
	mkdir -p target/workload-acceptance
	python3 tests/performance/paced.py --host target/debug/mas-host --count 4 --rate 20 --occupancy-ms 100 --concurrency 2 --fault-every 2 --fault-mode lost-completion --output target/workload-acceptance/lost-completion.json
	python3 tests/performance/paced.py --host target/debug/mas-host --count 3 --rate 2 --occupancy-ms 50 --concurrency 2 --fault-every 2 --fault-mode refuse-before-effect --output target/workload-acceptance/refusal.json

privacy-test:
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s adapters/privacy -p test_qualification.py -v
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=adapters/privacy python3 -m unittest discover -s adapters/privacy/tests -p 'test_*.py' -v
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests/privacy -p 'test_*.py' -v

notification-test:
	PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=sdk/python:adapters/notifications python3 -m unittest discover -s adapters/notifications/tests -p 'test_*.py' -v

governance-test:
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s deploy/managed/tests -v
	PYTHONDONTWRITEBYTECODE=1 MAS_HOST_BINARY="$${CARGO_TARGET_DIR:-target}/debug/mas-host" python3 -m unittest discover -s tests/governance -v

provider-test:
	node --test assets/providers/cursor.test.mjs

build:
	cargo build --locked -p mas-host

check:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings
	@test -z "$$(gofmt -l sdk/go tests/fixtures/go-release)" || { echo 'Run gofmt on Go source.' >&2; exit 1; }
	cd sdk/go && go vet ./...
	cd tests/fixtures/go-release && go vet ./...

test: check capability-test
	cargo test --locked --workspace
	cd sdk/go && go test -race ./...
	cd tests/fixtures/go-release && go test -race ./...
	mkdir -p target
	c++ -std=c++17 -Wall -Wextra -Werror -pthread -Isdk/cpp/include sdk/cpp/tests/conformance.cpp -o target/cpp-conformance
	target/cpp-conformance
	$(MAKE) python-test
	$(MAKE) workload-test
	$(MAKE) privacy-test
	$(MAKE) notification-test
	$(MAKE) governance-test

acceptance:
	cargo test --locked -p mas-host --test acceptance -- --ignored --nocapture
