# SPDX-License-Identifier: GPL-3.0-or-later
.PHONY: format format-check smoke rust-check audit check build native android-bindings apk-debug apk-release-unsigned android-check

format:
	nixfmt flake.nix nix/module.nix
	cargo fmt --manifest-path companion/Cargo.toml

format-check:
	nixfmt --check flake.nix nix/module.nix
	cargo fmt --manifest-path companion/Cargo.toml -- --check

smoke:
	cargo --version
	rustc --version
	rustfmt --version
	cargo clippy --version

rust-check:
	cargo test --locked --manifest-path companion/Cargo.toml --all-targets -j 2
	cargo clippy --locked --manifest-path companion/Cargo.toml --all-targets --keep-going -j 2 -- -D warnings

# The frozen RustSec DB predates the two Oct-5 Iroh GHSAs. Nix separately
# checks every selected core/base/relay version against the source-backed pins.
audit:
	jq '.advisoryDatabase, .advisoryExclusions' third_party/iroh-ffi/source.json
	cargo audit --no-fetch --db "$$GOATR_ADVISORY_DB" --file companion/Cargo.lock --json
	cargo audit --no-fetch --db "$$GOATR_ADVISORY_DB" --file third_party/iroh-ffi/Cargo.lock --json

check: format-check smoke rust-check audit

build:
	nix build --cores 2 --max-jobs 1 .#goatr

native:
	nix build --cores 2 --max-jobs 1 .#iroh-android

# Run inside the Android Nix shell: Gradle never fetches or builds Rust.
android-bindings:
	gradle --max-workers=2 --no-parallel :iroh-jvm:test :iroh-android:assembleDebug --dependency-verification strict --stacktrace

apk-debug:
	nix run --cores 2 --max-jobs 1 .#apk-debug

apk-release-unsigned:
	nix run --cores 2 --max-jobs 1 .#apk-release-unsigned

android-check:
	gradle --max-workers=2 --no-parallel :core:jvmTest :app:testDebugUnitTest :terminal:testDebugUnitTest --dependency-verification strict
