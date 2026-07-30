SHELL := /bin/sh

CARGO ?= cargo
SWIFT ?= swift
PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
LAUNCH_AGENTS_DIR ?= $(HOME)/Library/LaunchAgents
LOG_DIR ?= $(HOME)/Library/Logs/vzctl
ACTIVATE ?= 1

.DEFAULT_GOAL := help

.PHONY: help build build-cli build-daemon ci release test test-cli test-daemon \
	fmt fmt-check doctor doctor-json sign-helper install clean

help: ## Verfügbare Targets anzeigen
	@awk 'BEGIN {FS = ":.*## "; print "vzctl targets:"} /^[a-zA-Z0-9_-]+:.*## / {printf "  %-14s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: build-cli build-daemon ## CLI, Supervisor und Helper bauen

build-cli: ## Rust-CLI bauen
	$(CARGO) build --workspace

build-daemon: ## Swift-Supervisor und -Helper bauen
	$(SWIFT) build --package-path daemon

ci: fmt-check test build ## Formatierung, Tests und Build prüfen

release: ## Release-Binaries bauen und ad-hoc signieren
	$(CARGO) build --workspace --release
	$(SWIFT) build --package-path daemon --configuration release
	codesign --force --sign - --entitlements daemon/VzHelper.entitlements daemon/.build/release/vz-helper
	codesign --force --sign - --entitlements daemon/VzHelper.entitlements daemon/.build/release/vz-supervisor

install: release ## Installation erstellen/aktualisieren und Supervisor neu starten
	PREFIX="$(PREFIX)" BINDIR="$(BINDIR)" \
		LAUNCH_AGENTS_DIR="$(LAUNCH_AGENTS_DIR)" LOG_DIR="$(LOG_DIR)" \
		ACTIVATE="$(ACTIVATE)" daemon/scripts/install.sh \
		target/release/vzctl \
		daemon/.build/release/vz-supervisor \
		daemon/.build/release/vz-helper

test: test-cli test-daemon ## Alle Tests ausführen

test-cli: ## Rust-Tests ausführen
	$(CARGO) test --workspace

test-daemon: ## Swift-Tests ausführen
	$(SWIFT) test --package-path daemon

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

clean: ## Rust- und Swift-Build-Artefakte entfernen
	$(CARGO) clean
	$(SWIFT) package --package-path daemon clean
