# SPDX-License-Identifier: GPL-3.0-or-later
.PHONY: format format-check smoke check

format:
	nixfmt flake.nix

format-check:
	nixfmt --check flake.nix

smoke:
	cargo --version
	rustc --version
	rustfmt --version
	cargo clippy --version

check: format-check smoke
