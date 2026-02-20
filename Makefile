.PHONY: build release test check lint fmt fmt-check install clean cover release-patch release-minor release-major

build:
	cargo build

release:
	cargo build --release

test:
	cargo test --all

check: fmt-check lint test

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

install:
	cargo install --path .

clean:
	cargo clean

cover:
	cargo llvm-cov --html --output-dir target/coverage/html
	@echo "Coverage report: target/coverage/html/index.html"

cover-open: cover
	open target/coverage/html/index.html

release-patch:
	@VERSION=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$VERSION | cut -d. -f1); \
	MINOR=$$(echo $$VERSION | cut -d. -f2); \
	PATCH=$$(echo $$VERSION | cut -d. -f3); \
	NEW="$$MAJOR.$$MINOR.$$((PATCH + 1))"; \
	sed -i '' "s/^version = \"$$VERSION\"/version = \"$$NEW\"/" Cargo.toml; \
	echo "$$VERSION → $$NEW"; \
	cargo build --release

release-minor:
	@VERSION=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$VERSION | cut -d. -f1); \
	MINOR=$$(echo $$VERSION | cut -d. -f2); \
	NEW="$$MAJOR.$$((MINOR + 1)).0"; \
	sed -i '' "s/^version = \"$$VERSION\"/version = \"$$NEW\"/" Cargo.toml; \
	echo "$$VERSION → $$NEW"; \
	cargo build --release

release-major:
	@VERSION=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$VERSION | cut -d. -f1); \
	NEW="$$((MAJOR + 1)).0.0"; \
	sed -i '' "s/^version = \"$$VERSION\"/version = \"$$NEW\"/" Cargo.toml; \
	echo "$$VERSION → $$NEW"; \
	cargo build --release
