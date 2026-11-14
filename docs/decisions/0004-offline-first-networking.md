# ADR 0004 — Offline-first networking: passive detection, explicit probe

Status: accepted (2026-11-14)

## Decision
- **Passive state** (no packets sent) from sysfs/procfs: interfaces, carrier,
  addresses (getifaddrs), IPv4/IPv6 default routes, resolv.conf, ARP entry
  of the gateway. Levels: OFFLINE / LOCAL LINK / ROUTE UP (unverified).
- **Active probe only on explicit request** (key `p`): one TCP connect per
  configured target (default 9.9.9.9:443, 1.1.1.1:443) and one DNS lookup
  (default nixos.org), each with a timeout (default 2.5 s, clamped 0.1–10 s),
  run on a worker thread. **Skipped entirely without a default route.**
  No retries, no loops. Targets are configurable (`resilience.probe`).
- NetworkManager is used (it is the most complete text-configurable stack,
  `nmtui`), with its connectivity check explicitly disabled.
- `systemd-timesyncd` is disabled by default (it would contact NTP servers);
  documented in OFFLINE_OPERATIONS.md how to enable it.
- No listening services by default; the VM test asserts `ss -tuln` is empty.

## Consequences
"ONLINE" is only ever shown after a successful probe, with its time; the
interface never claims Internet access it has not verified.
