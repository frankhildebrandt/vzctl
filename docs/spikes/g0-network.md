# G0 Spike: Netzwerk / DNS / Crash

> Gate vor P0-Scaffolding. Host: **macOS 26.5** (Build 25F71), arm64, Xcode 26.6.  
> Epic: [#1](https://github.com/frankhildebrandt/vzctl/issues/1) · Stories: #3 #4 #5 #6  
> Harness: [`spikes/g0`](../../spikes/g0) · ADRs: [0002](../adr/0002-process-ownership.md), [0003](../adr/0003-apply-state.md)

## Status

| Check | Status | Notes |
|---|---|---|
| Host = macOS 26+ | ✅ | G1 / ADR 0001 |
| Zwei `shared` vmnet + feste Subnets | ✅ | Phase A |
| Host-Bridge nach Interface-Start | ✅ | `.0` |
| Host↔Guest static IP | ✅ | Phase C |
| Guest→Host ICMP `.0` / `.1` | ✅ / ❌ | |
| Guest→Host **UDP** `.0:15353` | ✅ | DNS |
| Guest→Host TCP `.0` | ❌ | |
| Cross-Net via Router `.2` | ✅ | |
| Supervisor-Kill Semantik | ✅ | Phase D |
| Sleep/Wake Clock-Drift | 📝 | manuelle Prozedur; Alpha-Risiko |
| **Go / No-Go** | ✅ **Go** | Sleep follow-up / ADR 0002 |

## IP-Konvention

| Rolle | Adresse |
|---|---|
| Host bridge / gw / DNS | **`.0`** (UDP) |
| API 2nd | `.1` — unused |
| Router | **`.2`** |
| Guests | `.10+` |

## Phase D — Crash (2026-07-30)

Script: `scripts/phase-d-crash.sh [--guest]`

### Net-only (`kill -9` ohne Cleanup)

```
CRASH_READY subnet=10.90.1.0
RECREATE_SAME_FAIL  → FAILURE(1001)
RECREATE_FRESH_OK   → 10.91.1.0
CLEAN_STOP_RECREATE_BLOCKED while network_ref retained
```

### Mit Guest

```
PRE_KILL_GUEST_OK  10.93.1.10
kill -9 holder
POST_KILL_GUEST_DEAD
GUEST_DEAD 10.93.1.10
RECREATE_SAME_FAIL 10.93.1.0
RECREATE_FRESH_OK  10.94.1.0
```

**Implikation:** Monolith-Kill = VM+Net tot + Subnet-Leak. Produktionsmodell = Helper-Prozess (ADR 0002).  
`stop_interface` ohne Ref-Release hält Reservation.

### Sleep (nicht automatisiert)

Host-Sleep würde Agent/Session unterbrechen. Prozedur:

1. `G0Spike hold-crash --guest` → Guest up  
2. SSH: `date -u +%s` speichern (Host+Guest)  
3. Mac Sleep ≥ 2 min, Wake  
4. SSH erneut: Drift = guest_now - host_now vs. pre  
5. Erwartung: Guest driftet ohne Agent-time-sync  

Alpha: dokumentiertes Risiko bis manuell gemessen; Agent bekommt `time-sync` in P0.

## Go / No-Go — **Go**

| Frage | Antwort |
|---|---|
| Dual-Net / static IP / DNS-UDP / Router | Ja |
| Crash-Verhalten verstanden | Ja → Helper + Ref-Release Pflicht |
| Sleep gemessen | Nein — Prozedur + Alpha-Risiko |
| **Verdict** | **Go** für P0; ADR 0002+0003 **Accepted** |

## Harness

```bash
cd spikes/g0
./scripts/phase-d-crash.sh           # net-only kill
./scripts/phase-d-crash.sh --guest   # + VM dies with process
.build/debug/G0Spike dnsudp|router
```
