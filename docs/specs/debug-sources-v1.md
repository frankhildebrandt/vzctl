# Debug Data Sources v1

Read-only observability queries against Postgres, MySQL, and NATS declared in
`observability.dataSources`. Execution always uses `vm exec` in a configured
**via VM** (guest must have `psql`, `mysql`, and/or `nats` in `PATH`).

## Config

See [hypernetwork-v1.md](hypernetwork-v1.md) § Observability → dataSources.

Source `host` accepts `.vz.test` names, private RFC1918 IPv4 addresses, and
IPv4 loopback (`127.0.0.0/8`). Loopback refers to the selected **via VM**, not
the macOS host. For a PostgreSQL listener bound to `127.0.0.1:5432` inside
`main-node`, use `via: main-node`, `host: 127.0.0.1`, and `port: 5432`.
A valid existing `via` VM remains mandatory; per-entry `via` overrides must
also reference an existing VM. Public IP addresses remain rejected.

## CLI

```bash
vzctl debug sources list -C ./my-stack --format json
vzctl debug doctor -C ./my-stack
vzctl debug postgres query -C ./my-stack --source app -- "SELECT 1"
vzctl debug nats sub -C ./my-stack --subject 'events.>' --count 5 --timeout 10s
```

Exit `26`: debug exec failed or prerequisites missing.

## REST

Under `/v1/stacks/{id}/debug/*` — worker forwards to `vzctl debug …`.

## MCP

Tools: `debug_sources_list`, `debug_doctor`, `debug_postgres_query`,
`debug_mysql_query`, `debug_nats_*` — parameter `stack_path` is the stack
directory (`-C`).

## Guardrails

- SQL: single read-only statement (`SELECT`, `EXPLAIN`, `SHOW`, `WITH … SELECT`)
- NATS: list/subscribe/kv get|watch only — no publish or admin mutations
- Secrets: `passwordFile` / `tokenFile` / `credsFile` relative to config dir
