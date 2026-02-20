.PHONY: build release test check lint fmt install clean release-patch release-minor

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

release-patch:
	@VERSION=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$VERSION | cut -d. -f1); \
	MINOR=$$(echo $$VERSION | cut -d. -f2); \
	PATCH=$$(echo $$VERSION | cut -d. -f3); \
	NEW="$$MAJOR.$$MINOR.$$((PATCH + 1))"; \
	sed -i '' "s/^version = \"$$VERSION\"/version = \"$$NEW\"/" Cargo.toml; \
	cargo check --quiet 2>/dev/null; \
	echo "$$VERSION → $$NEW"

release-minor:
	@VERSION=$$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/'); \
	MAJOR=$$(echo $$VERSION | cut -d. -f1); \
	MINOR=$$(echo $$VERSION | cut -d. -f2); \
	NEW="$$MAJOR.$$((MINOR + 1)).0"; \
	sed -i '' "s/^version = \"$$VERSION\"/version = \"$$NEW\"/" Cargo.toml; \
	cargo check --quiet 2>/dev/null; \
	echo "$$VERSION → $$NEW"
