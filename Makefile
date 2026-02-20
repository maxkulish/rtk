.PHONY: build release test check lint fmt install clean

build:
	cargo build

release:
	cargo build --release

test:
	cargo test --all

check: fmt lint test

lint:
	cargo clippy --all-targets

fmt:
	cargo fmt --all

install:
	cargo install --path .

clean:
	cargo clean
