# OIDC v1 (Dex)

Status: v0.2<br>
Issues: [#46](https://github.com/frankhildebrandt/vzctl/issues/46),
[#43](https://github.com/frankhildebrandt/vzctl/issues/43)

## Scope

Embedded Dex as OIDC provider. vzctl only autoconfigures clients and injects
env into guests — it does not implement an IdP.

## Canonical issuer

```text
https://auth.svc.{project}.vz.test
```

Never `https://auth.localhost`. Token `iss` and Guest Discovery must match.

Dex listens on `127.0.0.1:5556` (configurable). Caddy terminates TLS for
`auth.svc.…` and reverse-proxies to Dex.

## Config

```yaml
oidc:
  enabled: true
  mode: embedded
  issuer: https://auth.svc.edge-dmz.vz.test
  listen: "127.0.0.1:5556"
  clients: auto
  passwordFile: .vzctl/oidc/passwords.bcrypt
```

## clients: auto

For each VM or ingress route with `requires: [oidc]`:

- Create a Dex static client (`id` = VM / short name)
- Redirect URIs from ingress hosts (`https://{host}/oauth2/callback`)
- Persist secrets under `.vzctl/oidc/` (gitignored) or
  `Application Support/vzctl/projects/{project}/oidc/`

Inject into guest:

```bash
OIDC_ISSUER=https://auth.svc.edge-dmz.vz.test
OIDC_CLIENT_ID=web
OIDC_CLIENT_SECRET=…
OIDC_REDIRECT_URI=https://web.svc.edge-dmz.vz.test/oauth2/callback
OIDC_CA_PATH=/etc/ssl/certs/ca-certificates.crt
```

## Dev users

Static passwords from `passwordFile` (bcrypt). No production passwords in Git.

## CLI

```bash
vzctl oidc status
vzctl oidc clients
vzctl oidc token [--client ID]
```

## Acceptance

- Discovery at `/.well-known/openid-configuration`
- Auth Code + PKCE via Host browser
- Guest validates tokens against issuer + Local CA
