# api.nix - HOPR api Rust package definitions
#
# Builds the hopr-api crate for multiple platforms using nix-lib builders.
# Source filtering, rev, and build arguments are all defined here.
{
  builders,
  nixLib,
  pkgs,
  self,
  lib,
}:
let
  fs = lib.fileset;
  root = ./../..;

  rev = toString (self.shortRev or self.dirtyShortRev);

  depsSrc = nixLib.mkDepsSrc {
    inherit root fs;
  };

  src = nixLib.mkSrc {
    inherit root fs;
  };

  testSrc = nixLib.mkTestSrc {
    inherit root fs;
    extraFiles = [
      (fs.fileFilter (file: file.hasExt "snap") root)
    ];
  };

  cargoToml = ../../Cargo.toml;

  buildArgs = {
    inherit
      src
      depsSrc
      rev
      cargoToml
      ;
    cargoExtraArgs = "--all-features";
  };

  buildLib = builder: args: builder.callPackage nixLib.mkRustLibrary (buildArgs // args);

  clippyDerivation = buildLib builders.local { runClippy = true; };

  # Reuse Clippy's dev-profile dependency artifacts for the standalone
  # `cargo check` validation.
  checkDerivation = clippyDerivation.overrideAttrs (_: {
    pname = "hopr-api-check";
    buildPhase = ''
      runHook preBuild
      cargo check --all-features
      runHook postBuild
    '';
    installPhase = ''
      mkdir -p "$out"
    '';
  });
in
{

  check = checkDerivation;

  clippy = clippyDerivation;

  doc = buildLib builders.local {
    runDoc = true;
  };

  deny = buildLib builders.local {
    runDeny = true;
  };

  test =
    (buildLib builders.local {
      src = testSrc;
      runTests = true;
      extraNativeBuildInputs = [ pkgs.cargo-nextest ];
    }).overrideAttrs
      (_: {
        checkPhase = ''
          cargo nextest run --workspace --all-features --no-fail-fast
        '';
      });

  # nextest does not run doctests
  test-doc =
    (buildLib builders.local {
      src = testSrc;
      runTests = true;
    }).overrideAttrs
      (_: {
        checkPhase = ''
          cargo test --doc --workspace --all-features
        '';
      });

  docs = builders.localNightly.callPackage nixLib.mkRustPackage (buildArgs // { buildDocs = true; });

  coverage = builders.localCoverage.callPackage nixLib.mkRustPackage {
    src = testSrc;
    inherit
      depsSrc
      cargoToml
      rev
      ;
    runCoverage = true;
    cargoExtraArgs = "--all-features";
  };

  # Cross-compiled rlib packages
  # Artifacts are available at: ./result/lib/libhopr_api.rlib
  lib-hopr-api-x86_64-linux = buildLib builders."x86_64-linux" { };
  lib-hopr-api-aarch64-linux = buildLib builders."aarch64-linux" { };
  lib-hopr-api-x86_64-darwin = buildLib builders."x86_64-darwin" { };
  lib-hopr-api-aarch64-darwin = buildLib builders."aarch64-darwin" { };
  lib-hopr-api = buildLib builders.local { };

}
