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
ACTIVATE ?= 1

CADDY_VENDOR := daemon/Vendor/caddy/caddy
DEX_VENDOR := daemon/Vendor/dex/dex

.DEFAULT_GOAL := help

.PHONY: help build build-cli build-daemon build-agent ci release \
	test test-cli test-daemon test-agent \
	fmt fmt-check doctor doctor-json sign-helper \
	vendor vendor-caddy vendor-dex install-vendor \
	validate validate-edge-dmz \
	ui-install ui-dev ui-build \
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
	codesign --force --sign - daemon/.build/release/vz-dns-bind

vendor: vendor-caddy vendor-dex ## Caddy- und Dex-Binaries fetchen (v0.2)

vendor-caddy: ## Gepinntes Caddy-Binary nach daemon/Vendor/caddy/
	./scripts/fetch-caddy.sh

vendor-dex: ## Gepinntes Dex-Binary nach daemon/Vendor/dex/
	./scripts/fetch-dex.sh

install-vendor: ## Vendor-Binaries nach Application Support/vzctl/bin/ kopieren
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

install: release ## Installation erstellen/aktualisieren und Supervisor neu starten
	PREFIX="$(PREFIX)" BINDIR="$(BINDIR)" \
		LAUNCH_AGENTS_DIR="$(LAUNCH_AGENTS_DIR)" LOG_DIR="$(LOG_DIR)" \
		ACTIVATE="$(ACTIVATE)" daemon/scripts/install.sh \
		target/release/vzctl \
		daemon/.build/release/vz-supervisor \
		daemon/.build/release/vz-helper \
		daemon/.build/release/vz-dns-bind
	@if [ -x "$(CADDY_VENDOR)" ] && [ -x "$(DEX_VENDOR)" ]; then \
		$(MAKE) install-vendor; \
	else \
		echo "note: skip install-vendor (run make vendor && make install-vendor for Ingress/OIDC)"; \
	fi
	@echo "note: guest DNS :53 → sudo $(BINDIR)/vzctl dns install-bind-helper"

test: test-cli test-daemon test-agent ## Alle Tests ausführen

test-cli: ## Rust-Tests ausführen
	$(CARGO) test --workspace

test-daemon: ## Swift-Tests ausführen
	$(SWIFT) test --package-path daemon

test-agent: ## Guest-Agent-Tests ausführen
	$(GO) test -C guest-agent ./...

validate: validate-edge-dmz ## hypernetwork-Beispiele validieren

validate-edge-dmz: ## examples/edge-dmz gegen Schema prüfen
	$(CARGO) run -q -p vzctl -- validate -C examples/edge-dmz --format json

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

ui-build: ui-install ## Tauri-UI bauen
	cd apps/vzctl-ui && $(NPM) run tauri:build

clean: ## Rust- und Swift-Build-Artefakte entfernen
	$(CARGO) clean
	$(SWIFT) package --package-path daemon clean
	rm -rf guest-agent/bin
