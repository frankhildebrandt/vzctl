# macOS-26 Network-Resilience-Lab

Nur auf einem markierten Labor-Mac ausführen. Das Harness ändert keine
LAN/WLAN-/VPN-, Locale- oder Zeitzonen-Konfiguration. Dadurch ist die
Ausgangskonfiguration zugleich die Wiederherstellungskonfiguration; externe
Captive-/VPN-Emulatoren müssen ihren Zustand per eigener Fixture sichern und
im Cleanup zurücksetzen.

```bash
make build
VZCTL_NETWORK_LAB=1 scripts/lab-network-resilience.sh baseline

# Operator: Ethernet trennen, WLAN verbinden
VZCTL_NETWORK_LAB=1 scripts/lab-network-resilience.sh check ethernet-to-wifi 30

# Operator: WLAN trennen, Ethernet verbinden; danach VPN an/aus
VZCTL_NETWORK_LAB=1 scripts/lab-network-resilience.sh check wifi-to-ethernet 30
VZCTL_NETWORK_LAB=1 scripts/lab-network-resilience.sh check vpn-split-dns 30

# ausschließlich auf dem markierten Mac
VZCTL_NETWORK_LAB=1 VZCTL_NETWORK_LAB_SLEEP=1 \
  scripts/lab-network-resilience.sh sleep
```

Für jede Matrix-Zeile zusätzlich prüfen:

- interne Langzeit-TCP-Verbindung über den wachen Wechsel ohne Abbruch;
- DNS, TLS, OIDC, Ingress, Port-Forward und Docker-Kontext;
- Host online + absichtlich defekter Guest-Egress wird `degraded`, nicht
  `offline` oder `captive`;
- Captive-Redirect wird `captive`, nach Freischaltung innerhalb 30 Sekunden
  `healthy`;
- absichtliche VPN-CIDR-Überlappung wird `conflict`, Config und Netzidentität
  bleiben unverändert;
- Sleep im selben Netz und Sleep in Netz A/Wake in Netz B: intern ≤10 Sekunden,
  neuer Egress ≤30 Sekunden;
- Deutschland/Schweiz: Locale/Zeitzone vor dem Check ändern; nur Time-Sync darf
  reagieren, Netzwerk-Epoch/Config nicht allein deshalb.

Das Harness speichert keine SSID und fragt keine öffentliche IP ab. Die
Artefakte enthalten Desired-/Runtime-Netze, Doctor, lokale Routen und Resolver.
Vor einer Weitergabe deshalb dennoch prüfen.
