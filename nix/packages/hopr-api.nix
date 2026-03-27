# api.nix - HOPR api Rust package definitions
#
# Builds the hopr-api crate for multiple platforms using nix-lib builders.
# Source filtering, rev, and build arguments are all defined here.
{
  builders,
  nixLib,
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
  };

  buildLib =
    builder: args:
    builder.callPackage nixLib.mkRustLibrary (
      {
        inherit
          src
          depsSrc
          cargoToml
          rev
          ;
      }
      // args
    );
in
{

  clippy = buildLib builders.local { runClippy = true; };

  unit-test = buildLib builders.local {
    src = testSrc;
    runTests = true;
    cargoExtraArgs = "--lib --all-features";
  };

  docs = buildLib builders.localNightly { buildDocs = true; };

  # Cross-compiled rlib packages
  # Artifacts are available at: ./result/lib/libhopr_api.rlib
  lib-hopr-api-x86_64-linux = buildLib builders."x86_64-linux" { };
  lib-hopr-api-aarch64-linux = buildLib builders."aarch64-linux" { };
  lib-hopr-api-x86_64-darwin = buildLib builders."x86_64-darwin" { };
  lib-hopr-api-aarch64-darwin = buildLib builders."aarch64-darwin" { };
  lib-hopr-api = buildLib builders.local { };

}
