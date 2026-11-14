# Offline operations

Resilience is designed for **no Internet, no DNS, no public servers**.

## What never touches the network

| Component | Behaviour |
|---|---|
| Resilience TUI | Reads network state from the kernel only. Contacts nothing unless you press `p` in Network status. |
| NetworkManager | Connectivity checking disabled. Configures links you ask for (and DHCP on cables you plug in). |
| Time | `systemd-timesyncd` disabled: no NTP traffic. Set the clock from the TUI (*Date & time*, administrator password, also writes the hardware clock; ADR 0009) or enable NTP (below). |
| Nix | No channels, no automatic updates. If an administrator runs a build while offline, binary-cache connections fail after 5 s instead of hanging; anything not already in the local store cannot be fetched. |
| ModemManager | Disabled by default. |
| SSH | Disabled; no listening sockets at all by default (tested). |

## Network states shown by the interface

| State | Meaning | How it is determined |
|---|---|---|
| OFFLINE | no connected interface | sysfs carrier/operstate |
| LOCAL LINK ONLY | link but no default route | + /proc/net/route, ipv6_route |
| ROUTE UP (UNVERIFIED) | default route exists | Internet *not* assumed |
| ONLINE (hh:mm:ss) | last on-demand probe succeeded | TCP connect + DNS lookup |
| ROUTE UP, NO INTERNET | last probe failed | |

The on-demand probe contacts `resilience.probe.targets` (default
`9.9.9.9:443`, `1.1.1.1:443`) and resolves `resilience.probe.dnsName`
(default `nixos.org`), each with a 2.5 s timeout; it is skipped entirely when
there is no default route. Change or empty these lists in your configuration
if you do not want to contact those operators.

## Enabling optional online features (administrator)

```nix
services.timesyncd.enable = true;            # NTP time synchronisation
networking.modemmanager.enable = true;       # 3G/4G/5G modems
resilience.probe.targets = [ "192.0.2.10:443" ];  # your own probe target
```

## Planned

- Resource packs from USB/HDD (docs/RESOURCE_PACKS.md), Phase 3.
- Offline system updates via signed `nix copy` bundles, Phase 8.
- Local messaging between Resilience machines without Internet, Phase 6.
