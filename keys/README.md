# Resilience public keys

Only **public** keys live here. The secret keys never enter this
repository or any Nix store: the owner keeps them offline.

| File | Key | Used for |
|---|---|---|
| `resilience-release.minisign.pub` | minisign, key ID `5CC1E51F72AC25E7` | Signs each release's `SHA256SUMS` on the download mirrors (`scripts/make-release.sh`) |
| `../nix/release-keys.nix` | Nix binary-cache key `resilience-release` | Signs offline update bundles; built into installed systems (ADR 0023) |

The two keys are separate on purpose: a leak of one does not let anyone
forge what the other protects.

Checking a download (minisign is also in Resilience: Security & cryptography
→ Check a signature):

```sh
minisign -V -p resilience-release.minisign.pub -m SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
```

The same public key is published on GitHub and on the mirrors; if the copies
ever differ, do not trust either and ask.
