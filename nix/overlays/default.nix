# Adds the Resilience packages to nixpkgs.
final: _prev: {
  resilience = final.callPackage ../packages/resilience { };
  resilience-session = final.callPackage ../packages/resilience-session { };
  resilience-handbook = final.callPackage ../packages/resilience-handbook { };
}
