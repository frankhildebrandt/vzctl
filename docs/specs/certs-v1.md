# Local CA v1

Status: v0.2<br>
Issues: [#45](https://github.com/frankhildebrandt/vzctl/issues/45),
[#43](https://github.com/frankhildebrandt/vzctl/issues/43),
[#81](https://github.com/frankhildebrandt/vzctl/issues/81)

## Scope

Host-local Certificate Authority for Ingress TLS and OIDC trust. Guests receive
the CA into their system trust store so `curl` / apps work without `-k`.

## Storage (Host)

```text
~/Library/Application Support/vzctl/ca/
  root/
    ca.crt
    ca.key          # mode 0600
    fingerprint     # sha256 hex of ca.crt DER
  issued/
    {san}/
      cert.pem
      key.pem       # mode 0600
      meta.json
  trust/
    vzctl-local.crt # copy of ca.crt for rollout
```

One global CA per host in v0.2 (not per project). Never commit these files.

## CLI

```bash
vzctl certs ca init
vzctl certs ca upgrade --output /absolute/new-private-candidate
vzctl certs ca install          # optional macOS Keychain trust
vzctl certs mint <san> [--san alias...]
vzctl certs fingerprint
```

`certs rollout` und `certs verify` sind keine eigenständigen implementierten
Operationen; beide liefern Exit `2`. Rollout erfolgt über `vzctl apply`,
Verifikation über `vzctl vm exec` mit `curl --fail` gegen die Ziel-URL.

`vzctl doctor` meldet `certs.host_trust` (WARN), wenn die CA existiert, aber
noch nicht in der Login-Keychain vertrauenswürdig ist. Fix: `certs ca install`.
Firefox/Zen brauchen zusätzlich `security.enterprise_roots.enabled` oder einen
manuellen CA-Import.

## Guest rollout

1. **Boot seed (NoCloud):** write
   `/usr/local/share/ca-certificates/vzctl-local.crt` and run
   `update-ca-certificates`; CA creation happens before VM creation so the seed
   is present on the first apply.
2. **Live:** Supervisor method `vm.agent.ca_inject` routes through the helper to
   Guest-Agent method `ca_inject` (capability `ca_inject`). The agent invokes the
   restricted root helper `/usr/local/lib/vzctl/ca-inject`, which validates the
   fingerprint, installs atomically, refreshes and verifies the system bundle.
   Existing sealed agents fall back to the equivalent `vm.exec` + `sudo` path.
3. Lockfile / SQLite stores the CA fingerprint; drift → reinject
   (`certs.onRotate: reinject`) or reboot (`reboot`).

The apply step attempts every desired VM, aggregates per-VM failures and fails
hard if both the primary method and its compatibility fallback fail. A partial
or skipped rollout must never be reported as successful.

Java trust stores are out of Alpha / Nice-to-have.

## Rotate

1. `vzctl certs ca init --force` (or rotate helper) writes a new root.
2. Re-mint leafs for all ingress hosts.
3. Reload Caddy with new PEMs.
4. Reinject CA into guests (or reboot per `onRotate`).

## Striktes Zertifikatsprofil

Neue Roots enthalten kritische CA-BasicConstraints, kritische KeyUsage mit
`keyCertSign`/`cRLSign`, SKI und AKI. Server-Leafs enthalten SANs, AKI passend zum
Root-SKI, `digitalSignature`, `serverAuth` und explizit `CA=false`. Die Regression
prüft die vollständige Kette und den Hostnamen mit OpenSSL 3+ und `-x509_strict`;
TLS-Strictness wird weder im Produkt noch in den Lab-Probes abgeschaltet.

Default-Ensure prüft bestehende Root-Signatur, Schlüsselzuordnung, Gültigkeit,
Profil sowie Fingerprint/Trustkopie. Inkompatible oder halbe Paare werden mit einer
handlungsfähigen Fehlermeldung abgelehnt, niemals still ersetzt. `certs mint`
verwendet vorhandene, nachweislich zugehörige Leaf-Schlüssel erneut. Ein gültiges
Leaf mit identischen SANs, passender Metadaten-Fingerprint und mehr als 30 Tagen
Restlaufzeit bleibt bytegleich. Ein abgelaufenes Leaf kann mit demselben Schlüssel
neu ausgestellt werden. Atomare Dateiersetzung verhindert Teil-PEMs; ein beim
allerersten Anlegen unterbrochenes Paar bleibt ausdrücklich reparaturbedürftig.

## Explizites Profil-Upgrade (F45)

`vzctl certs ca upgrade --output <new-private-directory>` ist **Staging**, keine
aktive Migration und keine Schlüsselrotation. Der Output-Parent muss existieren;
das Ziel muss neu sein und außerhalb des kanonischen aktiven State-Verzeichnisses
liegen. Ein vorhandenes Ziel (auch Symlink) wird nie überschrieben. Wiederholung
benötigt nach Prüfung ein anderes neues Ziel. `--force` ist hier ungültig.

Der Kandidat spiegelt `ca/root`, `ca/trust` und alle `ca/issued/<san>`-Paare.
Verzeichnisse sind `0700`, alle Dateien `0600`; private Schlüssel werden bytegleich
kopiert und niemals im JSON ausgegeben. Root-DER-Subject, SPKI, SKI und Gültigkeit
sowie Leaf-DER-Subject, SPKI, SANs und Gültigkeit bleiben nachweislich erhalten.
Nicht unterstützte/duplizierte Extensions oder nicht verlustfrei übernehmbare
Identitäten werden abgelehnt. Das Profil-Upgrade verlängert keine Zertifikate.

`manifest.json` mit `schema=vzctl.ca-upgrade/v1` wird erst nach vollständiger
Erzeugung und erneuter Prüfung des Quell-Snapshots atomar veröffentlicht. Ohne
Manifest ist ein nach Fehler/Abbruch verbliebener Kandidat unvollständig und darf
nicht promoviert werden. Das Manifest enthält:

- kanonischen `source_state`, `output`, Erzeugungszeit, `state=candidate`,
  `promoted=false` sowie die notwendigen Trust-/Reload-/Strict-Verify-Gates;
- pro öffentlicher Datei Quell- und Kandidaten-SHA256;
- pro Zertifikat alte/neue DER-Fingerprints und SHA256 des unveränderten SPKI;
- private Key-Pfade und `key_unchanged=true`, aber keine privaten Schlüssel oder
  privaten Schlüssel-Hashes.

Die Promotion ist ein **separater, explizit freizugebender Betriebsablauf**:

1. Vollständige öffentliche Trustkopien, alle betroffenen Projekte/VMs,
   Container-/Kubernetes-CA-Mounts, Keychain und aktive Edge-Prozesse inventarisieren.
   Aktiven Zustand privat sichern; jede Kandidatenkette und jeden SAN mit OpenSSL
   `verify -x509_strict -purpose sslserver -verify_hostname ... -CAfile ...` prüfen.
   Alle Quell-/Kandidaten-Hashes und Schlüsselidentitäten erneut binden.
2. Laufende Applies ausschließen und Controller während atomarer Promotion
   quieszieren. Den neuen Default-Ensure-Code **nicht vor** der koordinierten
   Root-Promotion in automatisch laufende Controller ausrollen: alte Roots würden
   sonst korrekt abgelehnt und bestehende Stacks vorzeitig degradiert.
3. Nur die geprüften PEM-/Metadaten-/Fingerprint-Dateien mit vorbereiteter
   Rücksicherung promovieren; Root- und Leaf-Schlüssel niemals neu erzeugen.
   Trust an Host und allen inventarisierten Consumers gezielt aktualisieren.
4. Caddy explizit neu laden beziehungsweise den freigegebenen Edge-Neustart
   durchführen. Identische `ingress.ensure`-Argumente garantieren nach bloßer
   PEM-Ersetzung **keinen** Reload. System-Trust allein aktualisiert keine explizit
   gemounteten CA-Dateien oder Kubernetes ConfigMaps.
5. Bestehende VM-/Net-Identitäten und alle alten/neuen TLS-Consumer ohne `-k` oder
   Strictness-Abschaltung prüfen. Erst mit erfolgreichen Belegen abgeschlossen
   melden. Bei Fehlern anhand der vorherigen Hashes öffentliche Dateien/Trust und
   aktive Edge-Konfiguration kontrolliert zurücksetzen; nie DB oder Schlüssel
   durch ungeprüfte Sicherungen ersetzen.

Der konkrete Host-/Lab-Rollout wird separat unter `docs/planing/` geführt. Ein
Kandidaten-Manifest allein ist kein Beleg für erfolgreiche Promotion, Trust-Rollout
oder laufende Dienste.
