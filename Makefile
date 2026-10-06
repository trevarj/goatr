# SPDX-License-Identifier: GPL-3.0-or-later
.PHONY: lint format format-check smoke check

export PYTHONPATH := $(CURDIR)/companion
export PYTHONNOUSERSITE := 1

lint:
	ruff check companion

format:
	ruff format companion
	nixfmt flake.nix

format-check:
	ruff format --check companion
	nixfmt --check flake.nix

smoke:
	python3 -c 'import goatr_companion, aiohttp, cryptography, segno'

check: lint format-check smoke
