# G0 Spike Harness

Minimal macOS-26 harness for the network Go/No-Go gate.

## Commands

```bash
./scripts/prepare-assets.sh          # Ubuntu raw + cidata ISOs (xorriso)

swift build
codesign --force --sign - --entitlements G0Spike.entitlements .build/debug/G0Spike

./scripts/run-nets.sh 30 activate    # Phase A2
.build/debug/G0Spike guests 360      # Phase C reachability
.build/debug/G0Spike dnsudp 300      # Guest→Host UDP/TCP on .0
.build/debug/G0Spike router 480      # Router-VM Cross-Net
```

Protocol: [`docs/spikes/g0-network.md`](../../docs/spikes/g0-network.md)

## Requirements

- macOS 26+, Xcode 26+
- `com.apple.security.virtualization` (ad-hoc codesign)
- Do **not** claim `com.apple.vm.networking` with ad-hoc signing
- `qemu-img`, `xorriso`, `sshpass` for Phase C+
