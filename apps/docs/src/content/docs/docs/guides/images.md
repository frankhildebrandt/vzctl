---
title: Images
description: ARM64-Cloud-Images pullen, bakken und sealen.
---

Workflow:

```text
pull → bake --tag → seal --tag
```

Zielplattform: **ARM64 Cloud/Server-Images** (keine Installer-ISOs).

## Pull

```bash
vzctl image list
vzctl image pull ubuntu-latest
```

Bekannte Aliase u. a. `ubuntu`, `debian`, `alpine`, `arch`, `fedora`, `rocky`,
`alma`, `opensuse`, `coreos`, `flatcar`, `photon`, `talos-latest`
(siehe Contract `docs/images/pull-contract-v1.md`).

## Bake & Seal

```bash
vzctl image bake ubuntu-latest --tag v1
vzctl image seal ubuntu-latest --tag v1
```

CLI und REST brauchen `--tag` / `body.tag`. Die Config pinnt sealed Artefakte über
`spec.images.*.tag` (`baked|sealed/<canonical>@<tag>.raw`).

Apply skippt Bake/Seal, wenn der Tag bereits sealed ist.

Bake/Seal nutzen lokales `virt-customize` oder die gepinnte Builder-VM-Appliance
(`scripts/build-builder-appliance.sh`). Progress: `VZCTL_PROGRESS` / Job-Log.

## Clones

Sealed Base nie writable öffnen. VMs sind APFS Linked Clones + eigene dataDisk;
Identity (machine-id, MAC, SSH-Host-Keys, cloud-init instance-id) wird pro Clone neu gesetzt.
