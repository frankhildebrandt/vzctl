Vendor binaries are fetched via scripts/fetch-caddy.sh, scripts/fetch-dex.sh
and scripts/fetch-qemu-img.sh.

`make vendor` only writes this repository directory. It never updates a running
user installation. Use the explicit `make install-vendor` step to copy these
artifacts into Application Support.

Packaging requires all three executable bundles, each with `VERSION` and
`LICENSE`. qemu-img must include every non-system dylib through relative loader
paths. `make package` builds these dependencies; release CI does the same without
calling an installation target.
