# Public keys of the Resilience release key (ADR 0023): Nix binary-cache
# keys, `name:base64`. Offline update bundles must be signed by one of them.
# The secret key never enters this repository or any Nix store: the owner
# keeps it offline and signs bundles with scripts/make-update-bundle.sh.
#
# Empty: no release key has been made yet, so installed systems built from
# this tree cannot take updates (the TUI does not offer them).
[ ]
