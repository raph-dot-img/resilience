# A "snakeoil" Nix signing key for the vm-update test ONLY (ADR 0023), like
# the test keys nixpkgs publishes for its own VM tests. It is public on
# purpose and protects nothing: the test system trusts it so that bundles
# can be signed when the test is built. It must never appear in
# nix/release-keys.nix (checked by tests/eval/security-invariants.nix).
{
  secret = "resilience-vmtest-snakeoil-1:y2TCFZaDyoNa4a8+8QJ+yy9hLJIWs9yo2NFh8yoMJyIshfYxi7g0fQkwl+dnITUmQCZTFTBkRQ/4kDXmsMkypQ==";
  public = "resilience-vmtest-snakeoil-1:LIX2MYu4NH0JMJfnZyE1JkAmUxUwZEUP+JA15rDJMqU=";
}
