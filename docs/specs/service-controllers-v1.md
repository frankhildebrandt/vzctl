# K3s und Dienst-Controller v1

Status: Implementierungsvertrag, 20.10.2026. Tracker: [#73](https://github.com/frankhildebrandt/vzctl/issues/73).
Die bestehenden VM-/Docker-Workflows bleiben gültig. Neue Konfiguration ist optional.

## Vom Nutzer festgelegter Umfang

- VM-, Docker- und K3s-Anwendungen erhalten dieselben deklarativen Dienste.
- Eigene Controller für HTTPS-Ingress, OIDC/OAuth2, PostgreSQL und S3.
- `hypernetwork.config.yaml` bleibt Source of Truth. Kubernetes-YAMLs gehören
  zum Stack und werden dort referenziert; zusätzlich können Kubernetes-Ressourcen
  Dienstbedarf anmelden.
- CLI, REST/MCP, strukturierte Logs, Events und Diagnose müssen alle Zustände
  einschließlich Fehlern und Wiederanlauf für Agents zugänglich machen.

## Kubernetes-Cluster

```yaml
spec:
  kubernetes:
    clusters:
      dev:
        version: v1.x.y+k3sN # echte, explizit gepinnte Releaseversion
        sha256: <sha256-des-arm64-binaries>
        server: kube-server
        agents: [kube-worker]
        network: lan
        podCIDR: 10.42.0.0/16
        serviceCIDR: 10.43.0.0/16
        defaultNamespace: app
        manifests:
          - kubernetes/namespace.yaml
          - kubernetes/workloads.yaml
          - kubernetes/service-claims.yaml
```

`server`/`agents` sind vorhandene VM-Schlüssel, `network` ein vorhandenes Netz,
mit dem alle Knoten verbunden sind. Ein Knoten gehört höchstens zu einem Cluster.
Version und Binär-Hash sind erforderlich. CIDRs dürfen weder miteinander noch
mit Hypernetwork-Netzen kollidieren. v1 unterstützt einen Server mit optionalen
Agents auf systemd-basierten Linux-ARM64-Gästen; kein implizites HA-Versprechen.

Manifestpfade sind lokal, relativ zum Stack-Verzeichnis oder explizit absolut;
keine Remote-URLs und keine Shell-Auswertung. Mehrdokument-YAML ist zulässig.
Fehlende oder ungültige Dateien scheitern in Validate/Plan vor einer Mutation.
Dateiinhalt und vollständige Clusterkonfiguration beeinflussen Diff/Resume.
Das Lesen muss auf Größe und Dokumentanzahl begrenzt sein.

Apply wartet auf Guest-Agent/Netz, installiert das gepinnte ARM64-Binary mit
Hashprüfung, schreibt eine eigene systemd-Unit und K3s-Konfiguration und wartet
begrenzt auf API/Node-Readiness. Tokens werden zufällig erzeugt und im privaten
Stack-State verwahrt; Dateien mit Credentials haben Mode 0600. Kein `curl | sh`.
Die gebündelten Traefik-/ServiceLB-Komponenten werden deaktiviert, damit die
explizite vzctl-Ingress-Zuweisung die Host-Ports verwaltet. Die Kubernetes-API
wird nicht automatisch auf öffentlichen Host-Adressen freigegeben.

Bei aktivem Stack-DNS pinnt vzctl auf Server und Agents eine eigene Resolverdatei
`/etc/rancher/k3s/vzctl-resolv.conf` (0600) über K3s `resolv-conf`. Der Nameserver
ist die Bridge-Adresse `.0` des expliziten `cluster.network` im selben Projekt;
Ingress `.1`, Router `.2` und ein anderes zuerst gefundenes VM-Netz werden nicht
verwendet. Mehrdeutige Attachments, fremde Resolverdateien, Symlinks und nicht
brauchbare Unicast-Netze scheitern vor dem Überschreiben. Der Vertrag schränkt
bestehende gültige vmnet-CIDRs nicht zusätzlich auf RFC1918 ein.
Host-Resolverdateien und HostAliases bleiben unberührt.

Nach allen Node-Updates und Readiness migriert vzctl ausschließlich das von K3s
bereitgestellte CoreDNS-Deployment. Addon-Name, Namespace, Manifestquelle und
objectset-Eigentümerbindung werden geprüft; Addon-UID/ResourceVersion werden
vor dem Patch erneut gelesen. Der JSON-Patch testet Deployment-UID und
ResourceVersion und schreibt nur die eigene Pod-Template-Annotation
`vzctl.dev/k3s-resolver` (`v1:<OwnerSHA256>:<UpstreamSHA256>`).
Fremde Annotationseigentümer oder Identitätswechsel scheitern ohne Mutationretry.
Die erstmalige asynchrone Deploymentanlage wird begrenzt abgewartet; anschließend
werden Rollout und Identitäten erneut geprüft. Derselbe Hash löst keinen neuen
Rollout aus. Unveränderte Node-Dateien lösen keinen K3s-Dienstneustart aus.

Beim Deaktivieren von Stack-DNS entfernt vzctl zuerst die eigene `resolv-conf`-
Referenz, startet nur geänderte K3s-Dienste neu und entfernt danach die eigene
Resolverdatei. Ein zuvor gepinntes CoreDNS erhält einmal den eigenen Disabled-Hash;
ohne früheren Pin erfolgt keine DNS-Workloadmutation. Anwendungs-Pods und VMs
werden nicht neu erstellt. Bestehende Pods mit `dnsPolicy: Default` behalten
allerdings ihre Sandbox-Resolverkonfiguration bis zur regulären Neuerstellung.
Bestehende ClusterFirst-Pods nutzen nach dem CoreDNS-Rollout den korrigierten
Upstream ohne eigenen Recreate. Grundlage ist der dokumentierte
[K3s-Resolververtrag](https://docs.k3s.io/advanced#dns-resolution).

Manifeste werden serverseitig mit eigenem Field-Manager angewandt. Konflikte
werden gemeldet; keine erzwungene Übernahme fremder Felder. Verwalter-/Stack-/
Cluster-Labels erlauben Zuordnung und Inventar. Entfernte Manifeste sind im Diff
sichtbar, führen aber nicht zu impliziter Löschung von Daten oder Fremdobjekten.
Explizites Purge behält die bestehenden vzctl-Regeln bei.

Der imperative Erst-Create und SSA besitzen in Kubernetes auch bei identischem
Managernamen getrennte `Update`-/`Apply`-Einträge. Vor einer späteren Änderung
übergibt vzctl ausschließlich den deklarativ benötigten Teil seiner eigenen
Create-Felder an SSA: Felder aus dem aktuellen Soll sowie bereits vorhandene
eigene Apply-Felder. Auch aus dem Soll entfernte, zuvor per Apply verwaltete
Felder werden übergeben, damit SSA sie anschließend regulär entfernen kann.
Falls nötig etabliert eine unveränderte SSA-Projektion deren aktuelle Werte
anhand der eigenen `FieldsV1`-Einträge.
Dies unterstützt auch einen Erstlauf, der nach Create und vor SSA abbrach.
Fremde Felder werden nicht in diese Projektion aufgenommen. Keyed-/Set-Listen
werden anhand ihrer erfassten Schlüssel projiziert; unbekannte, mehrdeutige
oder nicht mehr vorhandene Feldstrukturen scheitern vor der Übergabe.
Bei granularen Map- und Listencontainern darf Kubernetes den Update-Strukturmarker
`.` in Apply weglassen, sofern sämtliche konkreten `f:`-Felder beziehungsweise
`k:`-/`v:`-Einträge abgedeckt sind. Dot-only- und atomare Knoten sowie der
Marker innerhalb eines Keyed-/Set-Listenelements bleiben streng geprüft;
dieser Unterschied ersetzt keine Feldbesitzprüfung. Maßgeblich sind der aktuelle
Werttyp und der Pfadkontext: Ein Objekt innerhalb einer Liste wird dadurch
nicht zum bloßen Mapcontainer. Unklare oder mehrdeutige Listenschlüssel bleiben
ein Fehler vor der Übergabe.

Erst nach vollständigem eigenem Apply-Besitz entfernt ein JSON-Patch genau
diesen Feldumfang aus den eigenen `Update`-Einträgen der aktuellen API-Version
und Hauptressource. Nicht deklarierte Serverdefaults bleiben im Update-Eintrag
erhalten; sie werden weder neu von Apply beansprucht noch später durch Weglassen
gelöscht. Ein vollständig geleerter eigener Update-Eintrag wird entfernt.
Der Patch testet UID, ResourceVersion und die vollständigen ManagedFields;
andere Manager, API-Versionen und Status-Subresources bleiben erhalten.
Ein anschließender GET prüft erneut Owner, UID, unveränderte Ressourcenwerte
und den exakt erhaltenen Feldbestand. Danach folgt die gewünschte SSA.
Es gibt weder `--force-conflicts` noch einen leeren/globalen ManagedFields-Reset.
Ein Konflikt während der Übergabe beendet den Versuch; fremder Feldbesitz
bleibt auch nach der Übergabe ein regulärer SSA-Konflikt.

`vzctl kubernetes status -C <stack> [--cluster <name>] --format json` liefert
Version, Knoten, Bedingungen, Manifestzustand und Fehler. Kubeconfig-Export
schreibt eine explizit angegebene Datei mit 0600; Schlüssel werden nicht in
Status/Logs/MCP-Antworten ausgegeben. Authentisierte Kubernetes-Zugriffe nutzen
TLS-Prüfung. Der Export ersetzt lediglich den Loopback-Serverendpunkt durch
die konfigurierte Knotenadresse, die als TLS-SAN eingerichtet wird.

## Einheitlicher Dienstbedarf

```yaml
spec:
  serviceProviders:
    database:
      kind: postgres
      via: db-vm
      endpoint: 127.0.0.1:5432
      advertiseEndpoint: db-vm.lan.demo.vz.test:5432
      adminUser: postgres
      passwordFile: ./secrets/postgres-admin
    objects:
      kind: s3
      via: docker-vm
      endpoint: http://127.0.0.1:9000
      advertiseEndpoint: http://docker-vm.lan.demo.vz.test:9000
      implementation: minio
      accessKeyFile: ./secrets/s3-admin-access
      secretKeyFile: ./secrets/s3-admin-secret
  serviceClaims:
    app-db:
      kind: postgres
      provider: database
      database: app
      consumers:
        - vm: app-vm
        - container: docker-vm/app
        - kubernetes: {cluster: dev, namespace: app, secret: app-db}
    app-files:
      kind: s3
      provider: objects
      bucket: app-files
      consumers:
        - kubernetes: {cluster: dev, namespace: app, secret: app-files}
    app-login:
      kind: oidc
      clientID: app
      redirectURIs: [https://app.svc.demo.vz.test/callback]
      consumers:
        - kubernetes: {cluster: dev, namespace: app, secret: app-login}
    app-web:
      kind: ingress
      host: app.svc.demo.vz.test
      backend: {vm: app-vm, port: 8080}
```

Diese Schnittstelle wird vor Controller-Implementierung im Schema fixiert;
Beispiele sind ein Vertrag, keine Behauptung bereits verfügbarer Funktionen.
Optionaler `provider` wird nur bei genau einem kompatiblen Provider automatisch
gewählt; mehrere Treffer sind ein erklärter Konflikt. Ingress und OIDC können
den bestehenden Stack-Provider verwenden. Externe Verwaltungscredentials
bleiben File-Referenzen. Kein Standardpasswort.

Jeder Claim besitzt eine stabile Identität aus Projekt/Name und einen eigenen
Desired-Hash. Controller sind unabhängig: ein PostgreSQL-Fehler blockiert keine
Ingress-Reparatur. Reconcile ist idempotent, besitzt eine Lease und eine begrenzte
Operationstimeout. Unveränderter Sollzustand verursacht keine Rotation,
Workload-Neustarts oder Ressourcen-Neuanlage. Wiederholungen nach Absturz sind
sicher. Fremde Datenbanken, Benutzer, Buckets und Clients werden nicht still
übernommen. Ressourcen-/Namenskonflikte liefern `OwnershipConflict`.

Daten und Credentials werden beim Entfernen eines Claims zunächst erhalten.
Explizites Löschen von Datenbanken/Buckets ist getrennt vom normalen Reconcile
und in der UI bestätigt. Ingress-Routen und OIDC-Clientregistrierungen folgen
dem aktuellen Sollzustand; entfernte Einträge werden beim nächsten Reconcile
nicht weiter veröffentlicht. Bereits gespeicherte Credentials bleiben erhalten.
Bei vorübergehend nicht erreichbarem Kubernetes-Backend bleibt die letzte
verifizierte Route desselben Claims und Hosts erhalten; Status wird Degraded.
Admin-Endpunkt und Consumer-Endpunkt sind getrennt: Bei Loopback-Admin-Endpunkt
und Verbrauchern außerhalb der Provider-VM ist `advertiseEndpoint` erforderlich.
Eine Adresse wird nicht allein durch DNS-Umschreibung als erreichbar angenommen.
S3-IAM-Provisionierung unterstützt in v1 ausdrücklich MinIO mit `mc`; andere
S3-Implementierungen benötigen einen eigenen IAM-Adapter und werden klar abgelehnt.

Pro `mc`-Aufruf werden stdout und stderr bereits beim Lesen gemeinsam auf
1 MiB begrenzt; stderr wird nicht gespeichert. Bei Überschreitung
(`InvalidProviderResponse`) oder Timeout (`ProviderTimeout`) wird die eigene
Prozessgruppe kontrolliert beendet und der gestartete Prozess eingesammelt.
Die Fristen bleiben 20 Sekunden pro Aufruf und 110 Sekunden für das Reconcile.
Fehler enthalten keine rohe Providerdiagnostik, Argumente oder Credentials.
Zur Zuordnung ergänzt der Controller ausschließlich validierte, feste
`operation`-Tokens im Fehlerhinweis: `bucket.list`, `account.list`,
`account.info`, `bucket.tags.read`, `bucket.create`, `bucket.tags.write`,
`account.create`, `account.policy.update` und `credentials.check`.
Unbekannte Tokens werden verworfen; die bestehenden Fehlercodes bleiben gültig.

Secret-Rotation ist eine explizite Operation mit nachweisbarer Verteilung.
Die eingebauten OIDC-Provider unterstützen `scopes` leer (Standard) oder genau
`[openid, profile, email]`. Sie besitzen keine Client-spezifische Scope-ACL;
andere Mengen werden mit `UnsupportedOidcScopes` abgewiesen. Client-IDs und
Redirect-Änderungen behalten bestehende Secrets. Der Marker `vzctlOwner`
verhindert die Übernahme automatisch angelegter oder fremder Clients.

### Controller-Verantwortung

| Controller | Verwaltetes Ergebnis | Bereitgestellte Binding-Felder |
|---|---|---|
| HTTPS-Ingress | Hostroute, TLS, Backend-Zuordnung | URL, CA-Dateireferenz |
| OIDC/OAuth2 | Client, Redirect-URIs, Scopes | issuer, client ID, Secret-Referenz |
| PostgreSQL | Datenbank, begrenzter Anwendungsbenutzer, Grants | host, port, database, user, Secret-Referenz |
| S3 | Bucket, begrenzte Policy und Anwendungszugang | endpoint, bucket, region, Secret-Referenzen |

VM-Bindings werden in einem privaten, verwalteten Gastverzeichnis atomar
bereitgestellt: `/var/lib/vzctl/bindings/<claim>/binding.json`. Docker erhält deklarative Bindings über die vorhandene
Container-/Compose-Konfiguration; notwendige Recreates müssen im Plan sichtbar
sein. Jedes Claim-Verzeichnis wird schreibgeschützt unter
`/run/vzctl/bindings/<claim>/` eingehängt; atomische Dateiersetzung bleibt dadurch
ohne Container-Neustart sichtbar. Der Host-Elternordner bleibt 0700. Für Container lesbare Claim-Verzeichnisse
sind im einzelnen Mount 0755 und deren Datei 0444; andere VM-Benutzer können
den privaten Elternordner nicht passieren. VM-only-Bindings bleiben 0700/0600.
Anwendungen lesen `binding.json` mit
`public` und `secrets`; vorhandene Prozess-Umgebungsvariablen ändern sich nicht. Kubernetes erhält namespace-lokale Secrets mit Ownership-Labels. Keine
Credentials in Labels, Events, Logs, Plan-Ausgaben oder ungeschützten URLs.

Kubernetes-Ressourcen unter `services.vzctl.dev/v1alpha1` spiegeln die vier
Claim-Typen. `metadata.uid` und Stack-/Cluster-Zuordnung bestimmen Ownership.
Ein Kubernetes-Claim darf keine Host-Dateien oder fremde Stacks referenzieren.
Secrets verbleiben im Namespace des Claims. Status besitzt `observedGeneration`,
`conditions`, referenzierte Bindings und einen maschinenlesbaren Fehlercode.
YAML- und Kubernetes-Claims benutzen dieselben Controller, keine zweite
Implementierung des Provisionierens. Doppelte Identitäten werden abgewiesen.

## Agent-Vertrag und Abnahme

- Status: `Pending`, `Reconciling`, `Ready`, `Degraded`, `Conflict`, `Retained`;
  jeweils `observedGeneration`, letzter erfolgreicher Lauf, Fehlercode/Hint.
- `controller.status.changed` trägt Projekt, Registrierungsgeneration und eine
  UUID-Laufkorrelation; Claim-Summaries enthalten `observedGeneration` und bei
  Kubernetes die Quelle (`cluster`, `namespace`, `name`, `uid`). Reine Zeitänderungen
  erzeugen kein Event. Felder sind skalare, längenbegrenzte Allowlist-Werte.
  Keine Secretwerte. Maschinenlesbares JSON folgt dem CLI-Contract.
- Read-only MCP: Inventar, Status, Logs, Diagnose. Mutationen sind ausdrücklich
  benannte Apply/Reconcile/Rotate/Delete-Operationen.
- Tests: Schema-/Roundtrip-Fixtures, Provider-Konflikte, Ownership, Secret-
  Redaktion, Wiederholung/Absturz/Wiederanlauf und Offline-Provider.
- Endabnahme auf isoliertem Lab: bestehender VM-/Docker-Workflow sowie ein K3s-
  Workload konsumieren DB/S3/OIDC und sind über HTTPS erreichbar. Ein Agent kann
  aus CLI/MCP-Status jeden Ausfall zuordnen und die Wiederherstellung bestätigen.

## Quellen

K3s-Konfigurationsdatei und Binary-Betrieb folgen der offiziellen
[Konfiguration](https://docs.k3s.io/installation/configuration).
Der privilegierte Kubeconfig-Zugriff folgt
[Cluster Access](https://docs.k3s.io/cluster-access); integrierte Komponenten
werden gemäß [Packaged Components](https://docs.k3s.io/installation/packaged-components)
explizit gesteuert. Die Controller-/Binding-Architektur ist eine vzctl-Entscheidung.
