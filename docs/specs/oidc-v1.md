# OIDC v1 (Dex + oidc-simple)

Status: v0.2<br>
Issues: [#46](https://github.com/frankhildebrandt/vzctl/issues/46),
[#43](https://github.com/frankhildebrandt/vzctl/issues/43)

## Scope

vzctl autoconfigures OIDC clients and injects env into guests. The IdP is one of:

- `mode: embedded` — Dex (default)
- `mode: oidc-simple` — **dev-only** click-to-login IdP (`vzctl-oidc-simple`)

## Canonical issuer

```text
https://auth.svc.{project}.vz.test
```

Never `https://auth.localhost`. Token `iss` and Guest Discovery must match.

The IdP listens on `127.0.0.1:5556` (configurable). Caddy terminates TLS for
`auth.svc.…` and reverse-proxies to the IdP.

`vz-edge` owns the Dex/oidc-simple child process. It keeps the applied IdP
running across control-plane restarts, restarts unexpected exits with bounded
backoff and restores the last-good runtime manifest after its own restart.

## Config — embedded Dex

```yaml
oidc:
  enabled: true
  mode: embedded
  issuer: https://auth.svc.edge-dmz.vz.test
  listen: "127.0.0.1:5556"
  clients: auto
  passwordFile: .vzctl/oidc/passwords.bcrypt
```

## Config — oidc-simple (dev)

No passwords. Users pick a name from a list and click login. Browser session
cookie enables logout via `end_session_endpoint`. **Not for production.**

Reference example [`examples/edge-dmz`](../../examples/edge-dmz) uses this mode.

```yaml
oidc:
  enabled: true
  mode: oidc-simple
  issuer: https://auth.svc.edge-dmz.vz.test
  listen: "127.0.0.1:5556"
  clients: auto
  users:
    - username: alice
      email: alice@dev.local
      role: admin              # optional custom claims (any YAML keys)
      teams: [platform]
    - username: bob
      email: bob@dev.local
```

Rules:

- `users` required (min. 1); each entry needs `username` + `email`
- Extra keys become token/userinfo claims alongside `sub`, `preferred_username`,
  `email`, `email_verified`
- `passwordFile` and `uplink` are **forbidden** with `oidc-simple`
- `users` is **forbidden** with `mode: embedded`

Binary: `vzctl-oidc-simple --config <runtime>/config.json` (installed under
`Application Support/vzctl/bin/` via `make install`).

Endpoints: discovery, JWKS, `/authorize` (picker), `/token` (Auth Code + PKCE),
`/userinfo`, `/end_session` (clears session cookie).

## clients: auto

For each VM or ingress route with `requires: [oidc]`:

- Create a static client (`id` = VM / short name)
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

## Dev users (embedded)

Static passwords from `passwordFile` (bcrypt). No production passwords in Git.
Without `passwordFile`/`uplink`, Dex ships a default `admin` / `password`.

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
- `oidc-simple`: picker login, custom claims in id_token, logout clears session
