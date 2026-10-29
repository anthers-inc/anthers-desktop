# ─── Anthers Desktop Makefile ───
#
# One crate, one resolved web build, one release flow. The shell is Rust in src-tauri/;
# ./web-dist is the platform's BUILD, put there by scripts/resolve-web.ts and never
# committed; the four version fields are moved together by scripts/promote-version.ts.
#
# The shape follows ~/Software/lily/Makefile — OS detection, a help target over the `##`
# descriptions, per-platform build targets that refuse politely on the wrong OS, and the
# macOS re-sign flow this repository's packaging lessons came from. `make verify` is the
# same step list .github/workflows/ci.yml runs, so a local green and a CI green mean the
# same thing.

.PHONY: help dev down web web-pinned web-ready setup install build build-unknown \
        build-linux build-windows build-macos notarize \
        check lint lint-fix format format-check test verify \
        version version-bump version-check clean

# ─── OS Detection ───
# Installers cannot be cross-compiled, so each per-platform target refuses rather than
# failing deep in a toolchain. Windows has no `uname`, so check its env var first.
ifdef OS
    ifeq ($(OS),Windows_NT)
        UNAME_S := Windows
    else
        UNAME_S := $(shell uname -s 2>/dev/null || echo Windows)
    endif
else
    UNAME_S := $(shell uname -s 2>/dev/null || echo Windows)
endif
ifneq (,$(findstring MINGW,$(UNAME_S)))
    DETECTED_OS := windows
else ifneq (,$(findstring MSYS,$(UNAME_S)))
    DETECTED_OS := windows
else ifneq (,$(findstring CYGWIN,$(UNAME_S)))
    DETECTED_OS := windows
else ifneq (,$(findstring Windows,$(UNAME_S)))
    DETECTED_OS := windows
else ifeq ($(UNAME_S),Linux)
    DETECTED_OS := linux
else ifeq ($(UNAME_S),Darwin)
    DETECTED_OS := macos
else
    DETECTED_OS := unknown
endif

# ─── Paths ───
# The shell is one crate and there is no frontend directory: `frontendDist` points at
# ../web-dist, which Tauri's before-commands fill. Everything Tauri writes lands under
# src-tauri/target, which is also what the release workflow uploads.
BACKEND := src-tauri
BUNDLE := $(BACKEND)/target/release/bundle
APP_NAME := Anthers
APP_BIN := anthers-desktop

# Windows gets PowerShell 7, matching Lily: complex recipes are one-liners there rather
# than POSIX shell, and `cd` inside one recipe line does not reach the next.
ifeq ($(DETECTED_OS),windows)
    SHELL := pwsh.exe
    .SHELLFLAGS := -NoProfile -Command
    BUN := bun
    # @tauri-apps/cli is a root devDependency, so Bun hoists its bin and `bunx tauri`
    # resolves from the repository root — which is where every Tauri command runs here.
    TAURI := bunx tauri
else
    BUN := bun
    TAURI := bunx tauri
endif

# 🚨 The digit in `a-zA-Z0-9` is load-bearing, and so is deduping by target name over
# the whole line: a target defined once per OS branch appears twice here, and two lines
# that differ only in their prerequisites survive `sort -u`. (The platform's help
# greps `[a-zA-Z_-]` and its `test-e2e*` targets silently never print.)
ifeq ($(DETECTED_OS),windows)
help: ## Show this help
	@Get-Content $(MAKEFILE_LIST) | ForEach-Object { if ($$_ -match '^([a-zA-Z0-9_.-]+):.*?## (.*)$$') { '{0,-18} {1}' -f $$Matches[1], $$Matches[2] } } | Sort-Object -Unique
else
help: ## Show this help
	@grep -E '^[a-zA-Z0-9_.-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort -u | awk 'BEGIN {FS = ":.*?## "}; !seen[$$1]++ {printf "\033[36m%-18s\033[0m %s\n", $$1, $$2}'
endif

# 🚨 A local build has no updater key, and `bundle.createUpdaterArtifacts: true` makes
# the bundler sign every artifact — so without TAURI_SIGNING_PRIVATE_KEY the run writes
# all three bundles and THEN exits with "A public key has been found, but no private
# key", which reads as a failed build when the installers are on disk. With no key in
# the environment, updater artifacts are turned off for this run; the release workflow
# sets the key and keeps them.
ifeq ($(TAURI_SIGNING_PRIVATE_KEY),)
    UPDATER_CONFIG := --config '{"bundle":{"createUpdaterArtifacts":false}}'
else
    UPDATER_CONFIG :=
endif

# ─── Run ───
# The window loads `devUrl` (http://localhost:3000) rather than the bundled dist, so the
# platform's dev web server has to be running beside it: `make dev` in ../Anthers. This
# repository never kills that server or frees that port — it is another project's
# process, and the machine-wide lesson is that a kill-loop pointed at a shared port takes
# whatever else happens to hold it.

ifeq ($(DETECTED_OS),windows)
dev: ## Start the Tauri dev shell (needs the platform's dev web server)
	@Write-Host "Starting the Anthers Desktop dev shell..."
	@Write-Host "  -> devUrl is http://localhost:3000: run the platform's 'make dev' in ..\Anthers first."
	$(TAURI) dev

down: ## Stop this checkout's dev server, and nothing else's
	@Write-Host "Close the app window, or end the terminal running 'make dev'."
else
dev: ## Start the Tauri dev shell (needs the platform's dev web server)
	@echo "Starting the Anthers Desktop dev shell..."
	@echo "  -> devUrl is http://localhost:3000: run the platform's 'make dev' in"
	@echo "     ../Anthers first. Nothing here kills that server — it belongs to"
	@echo "     another project, and the port is not this repository's to free."
	@echo "  -> Tauri's beforeDevCommand resolves the web app into ./web-dist first."
	$(TAURI) dev

down: ## Stop this checkout's dev server, and nothing else's
	@echo "Stopping the Anthers Desktop dev server..."
	@FOUND=0; \
	exe_of() { readlink /proc/$$1/exe 2>/dev/null || ps -o comm= -p $$1 2>/dev/null; }; \
	cwd_of() { readlink /proc/$$1/cwd 2>/dev/null || lsof -a -p $$1 -d cwd -Fn 2>/dev/null | sed -n 's/^n//p'; }; \
	ours() { case "$$1" in "$(CURDIR)"|"$(CURDIR)"/*) return 0 ;; *) return 1 ;; esac; }; \
	for PID in $$(pgrep -f '[t]arget/(debug|release)/$(APP_BIN)' 2>/dev/null); do \
		if [ "$$(basename "$$(exe_of $$PID)")" = "$(APP_BIN)" ] && ours "$$(cwd_of $$PID)"; then \
			kill $$PID 2>/dev/null && echo "  -> stopped the app window (pid $$PID)" && FOUND=1; \
		fi; \
	done; \
	for PID in $$(pgrep -f '[t]auri dev' 2>/dev/null); do \
		case "$$(basename "$$(exe_of $$PID)")" in \
			node|bun|bunx|node.exe|bun.exe) \
				if ours "$$(cwd_of $$PID)"; then \
					kill $$PID 2>/dev/null && echo "  -> stopped the dev CLI (pid $$PID)" && FOUND=1; \
				fi ;; \
		esac; \
	done; \
	[ "$$FOUND" = "1" ] || echo "  -> nothing running in this checkout."
endif

# ─── The web app ───
# scripts/resolve-web.ts picks its source in priority order: ANTHERS_WEB, a sibling
# ../Anthers checkout, then the release pinned in package.json. `web-pinned` forces the
# third, which is how the pin is exercised on a machine where the sibling exists.

web: ## Resolve the web app into ./web-dist (sibling checkout, else the pinned release)
	@$(BUN) run web

ifeq ($(DETECTED_OS),windows)
web-pinned: ## Resolve the pinned release asset, even when a sibling checkout exists
	@$$env:ANTHERS_WEB_SOURCE = "asset"; $(BUN) run web
else
web-pinned: ## Resolve the pinned release asset, even when a sibling checkout exists
	@ANTHERS_WEB_SOURCE=asset $(BUN) run web
endif

# ─── Setup ───

ifeq ($(DETECTED_OS),windows)
setup: ## Check Rust, Bun and the system dependencies, then bun install
	@Write-Host "================================================================================"
	@Write-Host "  Anthers Desktop setup - installing dependencies"
	@Write-Host "================================================================================"
	@if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) { Write-Host "Rust not found. Install from https://rustup.rs then re-run 'make setup'."; exit 1 } else { Write-Host "Rust: $$(rustc --version)" }
	@if (-not (Get-Command bun -ErrorAction SilentlyContinue)) { Write-Host "Bun not found. Install from https://bun.sh then re-run 'make setup'."; exit 1 } else { Write-Host "Bun: $$(bun --version)" }
	@Write-Host ""
	@Write-Host "Windows requirements for Tauri:"
	@Write-Host "  - Visual Studio C++ Build Tools"
	@Write-Host "  - WebView2 Runtime (pre-installed on Windows 10+)"
	@Write-Host "  See https://tauri.app/start/prerequisites/#windows"
	@Write-Host ""
	@Write-Host "Installing JS dependencies..."
	@$(BUN) install
	@Write-Host "Setup complete"
else
setup: ## Check Rust, Bun and the system dependencies, then bun install
	@echo "================================================================================"
	@echo "  Anthers Desktop setup - installing dependencies"
	@echo "================================================================================"
	@if ! command -v rustc >/dev/null 2>&1; then \
		echo "Rust not found. Installing via rustup..."; \
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh; \
		echo "Rust installed. Restart the shell and re-run 'make setup'."; \
		exit 0; \
	else \
		echo "Rust: $$(rustc --version)"; \
	fi
	@if ! command -v bun >/dev/null 2>&1; then \
		echo "Bun not found. Installing..."; \
		curl -fsSL https://bun.sh/install | bash; \
		echo "Bun installed. Restart the shell and re-run 'make setup'."; \
		exit 0; \
	else \
		echo "Bun: $$(bun --version)"; \
	fi
ifeq ($(DETECTED_OS),linux)
	@echo ""
	@echo "Checking the webview's Linux build dependencies (the set CI installs)..."
	@MISSING=""; \
	for p in libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev; do \
		if ! dpkg-query -W -f='$${Status}' "$$p" 2>/dev/null | grep -q "install ok installed"; then \
			MISSING="$$MISSING $$p"; \
		fi; \
	done; \
	if [ -n "$$MISSING" ]; then \
		echo "Missing system packages:$$MISSING"; \
		echo "  Install with: sudo apt install$$MISSING"; \
		echo "  Continuing with the JS install anyway — a build fails until these land."; \
	else \
		echo "All required system packages present."; \
	fi
else ifeq ($(DETECTED_OS),macos)
	@echo ""
	@echo "Checking macOS system dependencies..."
	@if ! xcode-select -p >/dev/null 2>&1; then \
		echo "Xcode Command Line Tools not installed."; \
		echo "  Install with: xcode-select --install"; \
	else \
		echo "Xcode Command Line Tools installed."; \
	fi
endif
	@echo ""
	@echo "Installing JS dependencies..."
	@$(BUN) install
	@echo "Setup complete"
endif

install: setup ## Install dependencies (alias for setup)

# ─── Build ───
# `build` is whatever this machine can produce; the per-platform targets carry the
# refusal. Output always lands in src-tauri/target/release/bundle/.

build: ## Build the installers for this platform
	@$(MAKE) --no-print-directory build-$(DETECTED_OS)

build-unknown:
	@echo "Unrecognized platform \"$(UNAME_S)\" — see the README's Packaging table."
	@exit 1

ifeq ($(DETECTED_OS),windows)
build-linux: ## Build Linux installers (.deb, .rpm, AppImage)
	@Write-Host "ERROR: Linux installers must be built on Linux."
	@exit 1

build-windows: ## Build the Windows installer (NSIS)
	@Write-Host "Building the Windows installer (.exe)..."
	@Write-Host "  -> Tauri's beforeBuildCommand resolves the web app first."
	$$env:PATH = "$$env:USERPROFILE\.cargo\bin;$$env:PATH"; $(TAURI) build --bundles nsis $(UPDATER_CONFIG)
	@Write-Host ""
	@Write-Host "Windows build complete — $(BUNDLE)/nsis/"

build-macos: ## Build macOS installers (.app, .dmg)
	@Write-Host "ERROR: macOS installers must be built on macOS."
	@exit 1

notarize: ## Submit the newest macOS DMG to Apple's notary service and staple it
	@Write-Host "ERROR: macOS notarization must be run on macOS."
	@exit 1
else
build-linux: ## Build Linux installers (.deb, .rpm, AppImage)
	@test "$(DETECTED_OS)" = linux || { echo "ERROR: Linux installers must be built on Linux (detected $(DETECTED_OS))."; exit 1; }
	@echo "Building Linux installers (.deb, .rpm, AppImage)..."
	@echo "  -> Tauri's beforeBuildCommand resolves the web app first."
	@$(TAURI) build --bundles deb,rpm,appimage $(UPDATER_CONFIG)
	@echo ""
	@echo "Linux build complete — $(BUNDLE)/"
	@echo "  - AppImage: $(BUNDLE)/appimage/"
	@echo "  - Debian:   $(BUNDLE)/deb/"
	@echo "  - RPM:      $(BUNDLE)/rpm/"

build-windows: ## Build the Windows installer (NSIS)
	@test "$(DETECTED_OS)" = windows || { echo "ERROR: Windows installers must be built on Windows (detected $(DETECTED_OS))."; exit 1; }
	@echo "Building the Windows installer (.exe)..."
	@$(TAURI) build --bundles nsis $(UPDATER_CONFIG)
	@echo ""
	@echo "Windows build complete — $(BUNDLE)/nsis/"

# Two things in this flow are not obvious, both learned from Lily (README § macOS
# signing + notarization): Tauri's bundler signs without `--timestamp`, which
# notarization rejects, so the build re-signs and then rebuilds the DMG from the
# re-signed .app; and APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID are unset while the
# bundler runs, because left set Tauri notarizes inline — a round trip to Apple on a
# binary that is about to be replaced.
build-macos: ## Build macOS installers (.app, .dmg)
	@test "$(DETECTED_OS)" = macos || { echo "ERROR: macOS installers must be built on macOS (detected $(DETECTED_OS))."; exit 1; }
	@if [ -f .env ]; then \
		echo "  -> signing config loaded from .env"; \
	else \
		echo "  -> WARNING: no .env — the build is unsigned and cannot be notarized"; \
		echo "     Copy .env.example to .env and fill in the four Apple values"; \
	fi
	@echo "  -> Building the .app and .dmg (Tauri resolves the web app first)..."
	@if [ -f .env ]; then \
		set -e; \
		. ./.env; \
		unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; \
		$(TAURI) build --bundles app,dmg $(UPDATER_CONFIG); \
	else \
		$(TAURI) build --bundles app,dmg $(UPDATER_CONFIG); \
	fi
	@if [ -f .env ]; then \
		set -e; \
		. ./.env; \
		APP="$(BUNDLE)/macos/$(APP_NAME).app"; \
		echo ""; \
		echo "Re-signing with hardened runtime and a secure timestamp..."; \
		echo "  (Tauri's bundler omits --timestamp; notarization rejects that)"; \
		codesign --force --options runtime --timestamp \
			--entitlements $(BACKEND)/entitlements.plist \
			--sign "$$APPLE_SIGNING_IDENTITY" \
			"$$APP/Contents/MacOS/$(APP_BIN)"; \
		codesign --force --options runtime --timestamp \
			--entitlements $(BACKEND)/entitlements.plist \
			--sign "$$APPLE_SIGNING_IDENTITY" \
			"$$APP"; \
		echo "  -> re-signed $$APP"; \
		echo ""; \
		echo "Rebuilding the DMG from the re-signed .app..."; \
		DMG=$$(ls -t "$(BUNDLE)/dmg/"*.dmg 2>/dev/null | head -1); \
		if [ -z "$$DMG" ]; then \
			echo "ERROR: no .dmg under $(BUNDLE)/dmg — the build did not produce one."; \
			exit 1; \
		fi; \
		rm -f "$$DMG"; \
		hdiutil create -volname "$(APP_NAME)" \
			-srcfolder "$$APP" -ov -format UDZO "$$DMG"; \
		codesign --force --timestamp --sign "$$APPLE_SIGNING_IDENTITY" "$$DMG"; \
		echo "  -> DMG rebuilt and signed: $$DMG"; \
		echo ""; \
		echo "Verifying the code signature..."; \
		codesign --verify --deep --strict "$$APP"; \
		echo "  -> code signature: OK"; \
	fi
	@echo ""
	@echo "macOS build complete — $(BUNDLE)/"
	@echo "  - App: $(BUNDLE)/macos/$(APP_NAME).app"
	@echo "  - DMG: $(BUNDLE)/dmg/"
	@if [ -f .env ]; then echo ""; echo "To notarize: make notarize"; fi

notarize: ## Submit the newest macOS DMG to Apple's notary service and staple it
	@test "$(DETECTED_OS)" = macos || { echo "ERROR: macOS notarization must be run on macOS (detected $(DETECTED_OS))."; exit 1; }
	@test -f .env || { echo "ERROR: .env is required for notarization."; echo "       Copy .env.example to .env and fill in APPLE_ID, APPLE_PASSWORD (an app-specific password), APPLE_TEAM_ID."; exit 1; }
	@set -e; \
	. ./.env; \
	DMG=$$(ls -t "$(BUNDLE)/dmg/"*.dmg 2>/dev/null | head -1); \
	if [ -z "$$DMG" ]; then \
		echo "ERROR: no .dmg under $(BUNDLE)/dmg — run 'make build-macos' first."; \
		exit 1; \
	fi; \
	echo "Submitting $$(basename $$DMG) for notarization..."; \
	xcrun notarytool submit "$$DMG" \
		--apple-id "$$APPLE_ID" \
		--password "$$APPLE_PASSWORD" \
		--team-id "$$APPLE_TEAM_ID" \
		--wait; \
	echo ""; \
	echo "Stapling the notarization ticket..."; \
	xcrun stapler staple "$$DMG"; \
	echo "  -> notarization complete: $$DMG"; \
	echo ""; \
	echo "Verifying the Gatekeeper assessment..."; \
	spctl --assess --type open --context context:primary-signature "$$DMG" 2>&1; \
	echo "  -> Gatekeeper: OK"
endif

# ─── Quality ───
# These four ARE ci.yml's Rust steps, in its order and with its flags. A local green
# that a red CI contradicts means one of them was run with different arguments.

ifeq ($(DETECTED_OS),windows)
check: ## Run the Rust compiler checks without building
	@Write-Host "Running Rust compiler checks..."
	cd $(BACKEND); cargo check
	@Write-Host "Rust checks passed"

lint: ## Check the Rust crate with clippy, warnings as errors
	@Write-Host "Linting the Rust crate..."
	cd $(BACKEND); cargo clippy -- -D warnings
	@Write-Host "Lint complete"

lint-fix: ## Apply clippy's automatic fixes
	@Write-Host "Applying clippy fixes..."
	cd $(BACKEND); cargo clippy --fix --all-targets --allow-dirty --allow-staged
	@Write-Host "Lint fix complete"

format: ## Format the Rust crate with rustfmt
	@Write-Host "Formatting the Rust crate..."
	cd $(BACKEND); cargo fmt
	@Write-Host "Format complete"

format-check: ## Refuse unformatted Rust (what CI runs)
	@Write-Host "Checking Rust formatting..."
	cd $(BACKEND); cargo fmt --check
	@Write-Host "Formatting is clean"

test: ## Run both suites (Rust + the scripts)
	@if (-not (Test-Path web-dist/index.html)) { Write-Host "  -> web-dist is missing; resolving the web app first"; & $(BUN) run web }
	@Write-Host "Running the Rust suite..."
	cd $(BACKEND); cargo test
	@Write-Host "Running the script suite..."
	@$(BUN) test
	@Write-Host "Both suites passed"
else
# The conf-path standing test asserts frontendDist exists at the base it resolves from,
# so the Rust suite needs a resolved ./web-dist. Resolving is a platform build or a
# signed download, so it happens only when the directory is absent — a fresh clone pays
# for it once, and every later run rides the copy already there.
web-ready:
	@test -f web-dist/index.html || { echo "  -> web-dist is missing; resolving the web app first"; $(MAKE) --no-print-directory web; }

check: ## Run the Rust compiler checks without building
	@echo "Running Rust compiler checks..."
	@cd $(BACKEND) && cargo check
	@echo "Rust checks passed"

lint: ## Check the Rust crate with clippy, warnings as errors
	@echo "Linting the Rust crate..."
	@cd $(BACKEND) && cargo clippy -- -D warnings
	@echo "Lint complete"

lint-fix: ## Apply clippy's automatic fixes
	@echo "Applying clippy fixes..."
	@cd $(BACKEND) && cargo clippy --fix --all-targets --allow-dirty --allow-staged
	@echo "Lint fix complete"

format: ## Format the Rust crate with rustfmt
	@echo "Formatting the Rust crate..."
	@cd $(BACKEND) && cargo fmt
	@echo "Format complete"

format-check: ## Refuse unformatted Rust (what CI runs)
	@echo "Checking Rust formatting..."
	@cd $(BACKEND) && cargo fmt --check
	@echo "Formatting is clean"

test: web-ready ## Run both suites (Rust + the scripts)
	@echo "Running the Rust suite..."
	@cd $(BACKEND) && cargo test
	@echo "Running the script suite..."
	@$(BUN) test
	@echo "Both suites passed"
endif

verify: ## Run what CI runs: formatting, clippy, both suites, the version invariant
	@$(MAKE) --no-print-directory format-check
	@$(MAKE) --no-print-directory lint
	@$(MAKE) --no-print-directory test
	@$(MAKE) --no-print-directory version-check
	@echo "Verify complete — green here means green in ci.yml."

# ─── Versioning ───
# One script owns all four fields (package.json, Cargo.toml, tauri.conf.json and the
# Cargo.lock entry). Hand-editing one of them ships a disagreement silently, because
# every tool reads its own copy and the build succeeds against whichever it found.

version: ## Show the version the four fields carry, and the next calver number
	@$(BUN) run scripts/promote-version.ts --check

version-check: ## Refuse a tree whose four version fields disagree (what CI runs)
	@$(BUN) run scripts/promote-version.ts --check

version-bump: ## Write the next calver version into all four fields (V=X.Y.N to pin)
	@$(BUN) run scripts/promote-version.ts $(if $(V),--version $(V),)

# ─── Maintenance ───

ifeq ($(DETECTED_OS),windows)
clean: ## Remove build artifacts and dependencies
	@Write-Host "Cleaning build artifacts..."
	@foreach ($$p in @("node_modules","web-dist",".web-dist.tar.gz","$(BACKEND)/target")) { if (Test-Path $$p) { Remove-Item -Recurse -Force $$p } }
	@Write-Host "Cleanup complete"
else
clean: ## Remove build artifacts and dependencies
	@echo "Cleaning build artifacts..."
	@rm -rf node_modules web-dist .web-dist.tar.gz $(BACKEND)/target
	@echo "Cleanup complete"
endif

.DEFAULT_GOAL := help
