# ADR 0001: macOS Baseline

- **Status:** Accepted
- **Date:** 2026-07-30
- **Issue:** [#2](https://github.com/frankhildebrandt/vzctl/issues/2)

## Context

vzctl benötigt Custom-vmnet-Topologien (`VZVmnetNetworkDeviceAttachment` und verwandte APIs). Pre-26-Pfade wären ein zweiter Produktmodus mit stillen Semantik-Unterschieden (SOL/Fable: kein transparenter Fallback).

## Decision

1. **Mindest-Host-OS: macOS 26** (Tahoe). Ältere Versionen sind **unsupported**.
2. **Bridged networking** bleibt in v0.1 **out of scope** (u. a. `com.apple.vm.networking` / Apple Approval).
3. `vzctl doctor` und Schema-Validierung sollen Host &lt; 26 als **hard fail** melden.

## Consequences

- G0-Spike und P0+ Implementierung zielen nur auf macOS 26+ APIs.
- Kein Pre-26 Compatibility-Layer im Alpha.
- Dokumentation und CI müssen macOS-26-Runner bzw. lokale 26er Hosts voraussetzen.
