# SPDX-License-Identifier: GPL-3.0-or-later
.PHONY: format format-check smoke rust-check audit check build native android-bindings studio

format:
	nixfmt flake.nix
	cargo fmt --manifest-path companion/Cargo.toml

format-check:
	nixfmt --check flake.nix
	cargo fmt --manifest-path companion/Cargo.toml -- --check

smoke:
	cargo --version
	rustc --version
	rustfmt --version
	cargo clippy --version

rust-check:
	cargo test --locked --manifest-path companion/Cargo.toml --all-targets
	cargo clippy --locked --manifest-path companion/Cargo.toml --all-targets -- -D warnings

# The frozen RustSec DB predates the two Oct-5 Iroh GHSAs. Nix separately
# checks every selected core/base/relay version against the source-backed pins.
audit:
	jq '.advisoryDatabase, .advisoryExclusions' third_party/iroh-ffi/source.json
	cargo audit --no-fetch --db "$$GOATR_ADVISORY_DB" --file companion/Cargo.lock --json
	cargo audit --no-fetch --db "$$GOATR_ADVISORY_DB" --file third_party/iroh-ffi/Cargo.lock --json

check: format-check smoke rust-check audit

build:
	nix build path:.#goatr

native:
	nix build path:.#iroh-android

# Run inside the Android Nix shell: Gradle never fetches or builds Rust.
android-bindings:
	bash ./gradlew :iroh-jvm:test :iroh-android:assembleDebug --dependency-verification strict --stacktrace

# Local design review only; never serve the repository root.
studio:
	darkhttpd studio --addr 127.0.0.1 --port 4173
