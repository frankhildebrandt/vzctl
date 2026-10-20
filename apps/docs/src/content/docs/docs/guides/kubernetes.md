---
title: Kubernetes und Dienst-Controller
description: K3s und lokale Manifeste pro Stack; gemeinsame Dienste für VM, Docker und Kubernetes.
---

Kubernetes-Dateien werden beim jeweiligen Stack in `hypernetwork.config.yaml`
angegeben. `spec.kubernetes` ist optional; vorhandene VM-/Docker-Stacks behalten
ihren bisherigen Ablauf. Der neue Funktionsumfang befindet sich in der
Release-Abnahme; die native Live-Prüfung ist noch nicht abgeschlossen.

## Cluster und Manifestdateien

```yaml
spec:
  kubernetes:
    clusters:
      dev:
        version: v1.37.1+k3s1
        sha256: a1561ca4aef8b99f5588a840d4468ac39c8c1cd92470bbcb9634823fa221741a
        server: server
        network: lan
        podCIDR: 10.42.0.0/16
        serviceCIDR: 10.43.0.0/16
        defaultNamespace: app
        manifests:
          - kubernetes/namespace.yaml
          - kubernetes/workloads.yaml
          - kubernetes/claims.yaml
```

`server` und optionale `agents` verweisen auf VM-Schlüssel desselben Stacks.
Jeder Knoten benötigt systemd, ARM64 und eine Adresse im angegebenen Netz.
Ein Cluster besitzt zunächst einen Server; HA wird nicht eingerichtet.
Version und passender SHA256 sind erforderlich. Der Installer prüft das Binary,
bevor er es ausführt. Die gezeigte Prüfsumme stammt aus dem offiziellen
[K3s-Release](https://github.com/k3s-io/k3s/releases/tag/v1.37.1%2Bk3s1).

Manifestpfade sind relativ zum Stack-Verzeichnis oder ausdrücklich absolut.
Mehrere YAML-Dokumente je Datei sind möglich. Remote-URLs werden abgelehnt.
Validate liest die Dateien; deren Inhalt geht in Plan, Diff und Resume ein.
Apply prüft Ownership, nutzt serverseitiges Apply und übernimmt fremde Ressourcen
nicht automatisch. Entfernte Manifestreferenzen löschen keine Ressourcen.

Ein vollständiges kleines Beispiel liegt unter `examples/k3s`.

```sh
vzctl validate -C examples/k3s
vzctl plan -C examples/k3s --format json
vzctl up -C examples/k3s
vzctl kubernetes status -C examples/k3s --format json
vzctl kubernetes kubeconfig -C examples/k3s --cluster dev --output /tmp/k3s-lab.yaml
```

Der Export erstellt eine neue private Kubeconfig-Datei mit Mode 0600.
Er gibt keine Schlüssel auf stdout aus. Die API ist über die Knotenadresse
mit TLS-Prüfung erreichbar; vzctl öffnet keinen öffentlichen Host-Port dafür.

## Dienste deklarieren

Ein Provider bezeichnet einen Dienst, den vzctl verwalten darf. Ein Claim
meldet den Bedarf einer Anwendung. Bei genau einem passenden Provider kann
`provider` entfallen; mehrere Treffer sind ein erklärter Konflikt.

```yaml
spec:
  serviceProviders:
    database:
      kind: postgres
      via: database-vm
      endpoint: 127.0.0.1:5432
      advertiseEndpoint: database-vm.lan.demo.vz.test:5432
      adminUser: postgres
      passwordFile: .secrets/postgres-admin
  serviceClaims:
    app-database:
      kind: postgres
      provider: database
      database: appdb
      consumers:
        - vm: application-vm
        - container: docker-vm/application
        - kubernetes:
            cluster: dev
            namespace: app
            secret: app-database
```

Provider-Credentials bleiben Dateireferenzen. Ein Loopback-Verwaltungsendpunkt
benötigt einen erreichbaren `advertiseEndpoint`, wenn andere VMs, Container oder
Kubernetes-Anwendungen den Dienst konsumieren. DNS allein schafft keine
Netzwerkfreigabe; Topologie und Policies müssen den Zugriff erlauben.

| Dienst | Ergebnis | Besonderheiten |
|---|---|---|
| `ingress` | HTTPS-Route und Zertifikat | Bestehender Stack-Edge; VM-Port oder Kubernetes Service |
| `oidc` | Client und Redirect-URIs | Bestehender Stack-IdP; stabile Secrets |
| `postgres` | Datenbank und begrenzte App-Rolle | Vorhandener Provider mit `psql` in seiner VM |
| `s3` | Bucket, Policy und App-Zugang | v1 unterstützt ausdrücklich MinIO mit `mc` und Python3 in seiner VM |

OIDC verwendet die Standard-Scopes `openid`, `profile`, `email`. Andere Mengen
werden abgewiesen, weil die eingebauten Provider keine Client-spezifische
Scope-Einschränkung bereitstellen. Admin- oder App-Secrets werden beim normalen
Reconcile nicht rotiert. Fremde Ressourcen werden nicht still übernommen.

## Kubernetes-Claims

Die vier CRDs `IngressClaim`, `OIDCClaim`, `PostgresClaim`, `S3Claim` verwenden
`apiVersion: services.vzctl.dev/v1alpha1`. Sie werden vor Benutzer-Manifests
installiert. Beispiel für den obigen Provider:

```yaml
apiVersion: services.vzctl.dev/v1alpha1
kind: PostgresClaim
metadata:
  name: app-database
  namespace: app
spec:
  provider: database
  database: appdb
  bindingSecret: app-database
```

Der erzeugte Secret bleibt im Namespace des Claims. Kubernetes-Claims dürfen
keine Host-Dateien, VM-Consumer oder fremden Namespaces angeben. Ingress-Backends
verweisen dort nur auf Services im eigenen Cluster und Namespace.
UID und Generation schützen Statusupdates vor versehentlich neu angelegten
Objekten gleichen Namens.

## Bindings und Betrieb

VMs erhalten `/var/lib/vzctl/bindings/<claim>/binding.json`. Docker mountet nur
das eigene Claim-Verzeichnis schreibgeschützt unter `/run/vzctl/bindings/<claim>/`.
Die Datei enthält `public` und `secrets`. Anwendungen lesen sie explizit;
bestehende Prozess-Umgebungsvariablen werden nicht nachträglich geändert.
Kubernetes-Anwendungen konsumieren die Schlüssel aus dem Binding-Secret.

```sh
vzctl controller status -C path/to/stack --format json
vzctl controller reconcile -C path/to/stack --format json
vzctl diagnose --output /tmp/new-diagnose.json --format json
```

Nach explizitem Apply registriert der Supervisor den Stack für periodisches
Reconcile. Down deaktiviert dies vor den VM-Mutationen. Ein laufender externer
Vorgang wird durch Abbruch nicht zurückgerollt. Status enthält Generation,
Bedingungen und Ursachen; Credentials erscheinen dort nicht. Änderungen des
Sollzustands machen bisheriges Ready zu Pending, bis erneut abgeglichen wurde.

Das Entfernen eines DB-/S3-Claims behält Daten und Credentials. Ingress-Routen und
OIDC-Registrierungen folgen dem aktuellen Sollzustand. Bei unvollständiger
Kubernetes-Discovery bleibt die bisherige Edge-Konfiguration erhalten und der
Status erklärt den Fehler. Das schützt bestehende Routen bei API-Ausfällen.
