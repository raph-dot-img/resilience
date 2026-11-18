# Public keys of the Resilience release key (ADR 0023): Nix binary-cache
# keys, `name:base64`. Offline update bundles must be signed by one of them.
# The secret key never enters this repository or any Nix store: the owner
# keeps it offline and signs bundles with scripts/make-update-bundle.sh.
#
# resilience-release: made by the owner on 2026-11-18. Systems built before
# this key was added (0.3.0 and earlier) cannot take updates.
[ "resilience-release:XNN+4i+Lxe69mUB0KLjui4U+gGNs5qhm2Q7v1yFR4R4=" ]
