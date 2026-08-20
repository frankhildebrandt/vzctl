SHELL := /bin/sh

CARGO ?= cargo
SWIFT ?= swift
GO ?= go
NPM ?= npm
PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
LAUNCH_AGENTS_DIR ?= $(HOME)/Library/LaunchAgents
LOG_DIR ?= $(HOME)/Library/Logs/vzctl
STATE_DIR ?= $(HOME)/Library/Application Support/vzctl
RUNTIME_BIN ?= $(STATE_DIR)/bin
APPLICATIONS_DIR ?= $(HOME)/Applications
ACTIVATE ?= 1

TAURI_APP := apps/vzctl-ui/src-tauri/target/release/bundle/macos/vzctl.app

CADDY_VENDOR := daemon/Vendor/caddy/caddy
DEX_VENDOR := daemon/Vendor/dex/dex
QEMU_IMG_VENDOR := daemon/Vendor/qemu-img/qemu-img
QEMU_IMG_LIBEXEC := $(STATE_DIR)/libexec/qemu-img

.DEFAULT_GOAL := help

.PHONY: help build build-cli build-daemon build-agent ci release \
	test test-cli test-daemon test-agent \
	fmt fmt-check doctor doctor-json sign-helper \
	vendor vendor-caddy vendor-dex vendor-qemu-img install-vendor install-qemu-img \
	validate validate-edge-dmz \
	smoke-split-dns \
	ui-install ui-dev ui-build package \
	install clean

help: ## Verfügbare Targets anzeigen
	@awk 'BEGIN {FS = ":.*## "; print "vzctl targets:"} /^[a-zA-Z0-9_-]+:.*## / {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: build-cli build-daemon ## CLI, Supervisor und Helper bauen

build-cli: ## Rust-CLI bauen
	$(CARGO) build --workspace

build-daemon: ## Swift-Supervisor und -Helper bauen
	$(SWIFT) build --package-path daemon

build-agent: ## Guest-Agent (Go) bauen
	$(GO) build -C guest-agent -o bin/vzctl-agent ./cmd/vzctl-agent

ci: fmt-check test build validate ## Formatierung, Tests, Build und Schema prüfen

release: ## Release-Binaries bauen und ad-hoc signieren
	$(CARGO) build --workspace --release
	$(SWIFT) build --package-path daemon --configuration release
	codesign --force --sign - --entitlements daemon/VzHelper.entitlements daemon/.build/release/vz-helper
	codesign --force --sign - --entitlements daemon/VzHelper.entitlements daemon/.build/release/vz-supervisor
	codesign --force --sign - --entitlements daemon/VzHelper.entitlements daemon/.build/release/vz-net
	codesign --force --sign - daemon/.build/release/vz-edge
	codesign --force --sign - daemon/.build/release/vz-dns-bind

vendor: vendor-caddy vendor-dex vendor-qemu-img ## Caddy-, Dex- und qemu-img-Binaries fetchen

vendor-caddy: ## Gepinntes Caddy-Binary nach daemon/Vendor/caddy/
	./scripts/fetch-caddy.sh

vendor-dex: ## Gepinntes Dex-Binary nach daemon/Vendor/dex/
	./scripts/fetch-dex.sh

vendor-qemu-img: ## Relokierbares qemu-img nach daemon/Vendor/qemu-img/
	./scripts/fetch-qemu-img.sh

install-vendor: ## Vendor-Binaries nach Application Support/vzctl kopieren
	@mkdir -p "$(RUNTIME_BIN)"
	@if [ -x "$(CADDY_VENDOR)" ]; then \
		install -m 755 "$(CADDY_VENDOR)" "$(RUNTIME_BIN)/caddy"; \
		echo "installed: $(RUNTIME_BIN)/caddy"; \
	else \
		echo "missing $(CADDY_VENDOR) — run: make vendor-caddy" >&2; \
		exit 3; \
	fi
	@if [ -x "$(DEX_VENDOR)" ]; then \
		install -m 755 "$(DEX_VENDOR)" "$(RUNTIME_BIN)/dex"; \
		echo "installed: $(RUNTIME_BIN)/dex"; \
	else \
		echo "missing $(DEX_VENDOR) — run: make vendor-dex" >&2; \
		exit 3; \
	fi
	$(MAKE) install-qemu-img

install-qemu-img: ## Vendored qemu-img nach Application Support/vzctl/libexec/ kopieren
	@if [ ! -x "$(QEMU_IMG_VENDOR)" ]; then \
		$(MAKE) vendor-qemu-img; \
	fi
	@test -x "$(QEMU_IMG_VENDOR)" || { echo "missing $(QEMU_IMG_VENDOR)" >&2; exit 3; }
	@mkdir -p "$(QEMU_IMG_LIBEXEC)"
	@ditto daemon/Vendor/qemu-img "$(QEMU_IMG_LIBEXEC)"
	@echo "installed: $(QEMU_IMG_LIBEXEC)/qemu-img"

install: release ui-build ## CLI, Daemons und Tauri-App installieren/aktualisieren
	PREFIX="$(PREFIX)" BINDIR="$(BINDIR)" \
		LAUNCH_AGENTS_DIR="$(LAUNCH_AGENTS_DIR)" LOG_DIR="$(LOG_DIR)" \
		ACTIVATE="$(ACTIVATE)" daemon/scripts/install.sh \
		target/release/vzctl \
		daemon/.build/release/vz-net \
		daemon/.build/release/vz-edge \
		daemon/.build/release/vz-supervisor \
		daemon/.build/release/vz-helper \
		daemon/.build/release/vz-dns-bind
	@mkdir -p "$(RUNTIME_BIN)"
	@if [ -x target/release/vzctl-oidc-simple ]; then \
		install -m 755 target/release/vzctl-oidc-simple "$(RUNTIME_BIN)/vzctl-oidc-simple"; \
		echo "installed: $(RUNTIME_BIN)/vzctl-oidc-simple"; \
	fi
	@if [ -x "$(CADDY_VENDOR)" ] && [ -x "$(DEX_VENDOR)" ]; then \
		$(MAKE) install-vendor; \
	else \
		echo "note: skip Caddy/Dex vendor (run make vendor && make install-vendor for Ingress/OIDC)"; \
		$(MAKE) install-qemu-img; \
	fi
	@test -d "$(TAURI_APP)" || { echo "missing Tauri app: $(TAURI_APP)" >&2; exit 3; }
	@mkdir -p "$(APPLICATIONS_DIR)"
	@ditto "$(TAURI_APP)" "$(APPLICATIONS_DIR)/vzctl.app"
	@echo "installed: $(APPLICATIONS_DIR)/vzctl.app"
	@echo "note: guest DNS :53 → sudo $(BINDIR)/vzctl dns install-bind-helper"

test: test-cli test-daemon test-agent ## Alle Tests ausführen

test-cli: ## Rust-Tests ausführen
	$(CARGO) test --workspace

test-daemon: ## Swift- und Daemon-Script-Tests ausführen
	$(SWIFT) test --package-path daemon
	daemon/scripts/test-stop-vz-helpers.sh

test-agent: ## Guest-Agent-Tests ausführen
	$(GO) test -C guest-agent ./...

validate: validate-edge-dmz ## hypernetwork-Beispiele validieren

validate-edge-dmz: ## examples/edge-dmz gegen Schema prüfen
	$(CARGO) run -q -p vzctl -- validate -C examples/edge-dmz --format json

smoke-split-dns: ## Privilegierter Split-Horizon Multi-Net/Docker-Smoke-Test
	scripts/smoke-split-horizon-dns.sh

fmt: ## Rust-Code formatieren
	$(CARGO) fmt --all

fmt-check: ## Rust-Formatierung prüfen
	$(CARGO) fmt --all --check

doctor: ## Host-Checks als Text ausführen
	$(CARGO) run -q -p vzctl -- doctor

doctor-json: ## Host-Checks als JSON ausführen
	$(CARGO) run -q -p vzctl -- doctor --format json

sign-helper: build-daemon ## Helper für lokale Entwicklung ad-hoc signieren
	daemon/scripts/codesign-helper.sh daemon/.build/debug/vz-helper

ui-install: ## Tauri-UI npm-Dependencies installieren
	cd apps/vzctl-ui && $(NPM) install

ui-dev: ui-install ## Tauri-UI im Dev-Modus starten (braucht vzctl auf PATH)
	cd apps/vzctl-ui && $(NPM) run tauri:dev

ui-build: ui-install ## Tauri-UI als Release-App bauen
	cd apps/vzctl-ui && $(NPM) run tauri:build -- --bundles app

package: release ui-build ## tar.gz + .pkg + .dmg unter dist/ erzeugen
	RELEASE_TAG="$(RELEASE_TAG)" ./scripts/package-macos.sh

clean: ## Rust- und Swift-Build-Artefakte entfernen
	$(CARGO) clean
	$(SWIFT) package --package-path daemon clean
	rm -rf guest-agent/bin dist
