.DEFAULT_GOAL := help

.PHONY: fmt lint test wasm serve check help

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

wasm:
	./scripts/build-wasm.sh

serve:
	python3 -m http.server 8080 --directory web

check: fmt lint test wasm

help:
	@echo "fmt    cargo fmt --all -- --check"
	@echo "lint   cargo clippy --workspace --all-targets -- -D warnings"
	@echo "test   cargo test --workspace"
	@echo "wasm   ./scripts/build-wasm.sh"
	@echo "serve  python3 -m http.server 8080 --directory web"
	@echo "check  fmt lint test wasm"
	@echo "help   print this list"
