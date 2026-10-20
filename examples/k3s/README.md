# Kubernetes-Dateien gehören zum Stack

Dieses Beispiel referenziert Namespace und ConfigMap direkt aus
`hypernetwork.config.yaml`. Relative Pfade beginnen im Stack-Verzeichnis;
Mehrdokument-YAML ist ebenfalls möglich. Änderungen der Dateien beeinflussen
Plan, Diff und Resume. Entfernte Dateien werden nicht automatisch gepruned.

```sh
vzctl validate -C examples/k3s
vzctl plan -C examples/k3s --format json
# Provisioniert eine eigene VM; benötigt ein passendes Ubuntu-Base-Image:
vzctl up -C examples/k3s
vzctl kubernetes status -C examples/k3s --format json
vzctl controller status -C examples/k3s --format json
vzctl kubernetes kubeconfig -C examples/k3s --cluster dev --output /tmp/k3s-lab-kubeconfig.yaml
```

Die Kubeconfig-Datei darf noch nicht existieren und wird privat mit 0600 angelegt.
Das API ist über die konfigurierte Knotenadresse erreichbar; kein automatischer
öffentlicher Host-Port. Dieser Beispielstand wurde auf Schema-/Vertragsebene
geprüft; die Live-Abnahme des neuen K3s-Lifecycle bleibt im Release-Tracker offen.

Version und ARM64-Prüfsumme stammen aus dem offiziellen
[K3s-Release v1.37.1+k3s1](https://github.com/k3s-io/k3s/releases/tag/v1.37.1%2Bk3s1)
und dessen [Prüfsummenliste](https://github.com/k3s-io/k3s/releases/download/v1.37.1%2Bk3s1/sha256sum-arm64.txt),
abgerufen am 20.10.2026. Ein Versionswechsel erfordert einen passenden neuen Hash;
der Installer prüft das heruntergeladene Binary vor der Ausführung.

Für Anwendungen weitere lokale YAML-Dateien unter `manifests` ergänzen.
Die vier Dienst-Claim-Typen werden vor Benutzer-Manifests installiert.
Provider müssen im Stack unter `serviceProviders` freigegeben sein. Ein Beispiel
für einen bereits konfigurierten PostgreSQL-Provider namens `database`:

```yaml
apiVersion: services.vzctl.dev/v1alpha1
kind: PostgresClaim
metadata:
  name: app-database
  namespace: app
spec:
  provider: database
  database: appdb
  bindingSecret: app-database-binding
```

Das Binding-Secret bleibt im Namespace des Claims. Die Anwendung muss die
benötigten Schlüssel aus diesem Secret konsumieren. Die übrigen Typen heißen
`S3Claim`, `OIDCClaim` und `IngressClaim`. Gemeinsame Verträge, Provider-Bedingungen,
VM-/Docker-Bindings und Aufbewahrung stehen in
[service-controllers-v1.md](../../docs/specs/service-controllers-v1.md).
