{
  description = "hopr-api — common high-level API traits for the HOPR protocol";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:NixOS/nixpkgs/release-25.11";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/master";
    rust-overlay.url = "github:oxalica/rust-overlay/master";
    crane.url = "github:ipetkov/crane/v0.23.0";

    # HOPR Nix Library (provides flake-utils and reusable build functions)
    nix-lib.url = "github:hoprnet/nix-lib/v1.1.0";
    pre-commit.url = "github:cachix/git-hooks.nix";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    flake-root.url = "github:srid/flake-root";

    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    nix-lib.inputs.nixpkgs.follows = "nixpkgs";
    nix-lib.inputs.flake-utils.follows = "flake-utils";
    nix-lib.inputs.crane.follows = "crane";
    nix-lib.inputs.flake-parts.follows = "flake-parts";
    nix-lib.inputs.rust-overlay.follows = "rust-overlay";
    nix-lib.inputs.treefmt-nix.follows = "treefmt-nix";
    nix-lib.inputs.nixpkgs-unstable.follows = "nixpkgs-unstable";
    pre-commit.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-unstable,
      flake-utils,
      flake-parts,
      rust-overlay,
      crane,
      nix-lib,
      pre-commit,
      ...
    }@inputs:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.treefmt-nix.flakeModule
        inputs.flake-root.flakeModule
      ];
      perSystem =
        {
          config,
          lib,
          system,
          ...
        }:
        let
          localSystem = system;
          overlays = [
            (import rust-overlay)
          ];
          pkgs = import nixpkgs { inherit localSystem overlays; };
          pkgs-unstable = import nixpkgs-unstable { inherit localSystem overlays; };

          # Platform information
          buildPlatform = pkgs.stdenv.buildPlatform;

          # Import nix-lib for shared Nix utilities
          nixLib = nix-lib.lib.${system};

          # Create all Rust builders for cross-compilation using nix-lib
          builders = nixLib.mkRustBuilders {
            inherit localSystem;
            rustToolchainFile = ./rust-toolchain.toml;
          };

          hoprApiPackages = import ./nix/packages/hopr-api.nix {
            inherit
              builders
              nixLib
              self
              lib
              ;
          };

          # Load nightly toolchain from file for rustfmt
          nightlyToolchain = pkgs.rust-bin.selectLatestNightlyWith (
            toolchain:
            toolchain.minimal.override {
              extensions = [ "rustfmt" ];
            }
          );

          # Wrapper for rustfmt to fix macOS dylib loading issue
          rustfmtWrapper = pkgs.writeShellScriptBin "rustfmt" ''
            export DYLD_LIBRARY_PATH="${nightlyToolchain}/lib:$DYLD_LIBRARY_PATH"
            exec "${nightlyToolchain}/bin/rustfmt" "$@"
          '';

          pre-commit-check = pre-commit.lib.${system}.run {
            src = ./.;
            hooks = {
              treefmt.enable = false;
              treefmt.package = config.treefmt.build.wrapper;
              check-executables-have-shebangs.enable = true;
              check-shebang-scripts-are-executable.enable = true;
              check-case-conflicts.enable = true;
              check-symlinks.enable = true;
              check-merge-conflicts.enable = true;
              check-added-large-files.enable = true;
              commitizen.enable = true;
              sync-copilot-instructions = {
                enable = true;
                name = "Sync .claude/ instructions to Copilot files";
                entry = "bash .github/scripts/sync-copilot-instructions.sh";
                files = "(\\.claude/INSTRUCTIONS\\.md|\\.claude/rust\\.md)";
                language = "system";
                pass_filenames = false;
              };
            };
            tools = pkgs;
          };

          devShell = nixLib.mkDevShell {
            rustToolchainFile = ./rust-toolchain.toml;
            shellName = "hopr-api Development";
            treefmtWrapper = config.treefmt.build.wrapper;
            treefmtPrograms = pkgs.lib.attrValues config.treefmt.build.programs;
            extraPackages = with pkgs; [
              pkgs-unstable.cargo-audit
              cargo-machete
              cargo-shear
              cargo-insta
            ];
            shellHook = ''
              ${pre-commit-check.shellHook}
            '';
          };

          ciShell = nixLib.mkDevShell {
            rustToolchainFile = ./rust-toolchain.toml;
            shellName = "hopr-api CI";
            treefmtWrapper = config.treefmt.build.wrapper;
            treefmtPrograms = pkgs.lib.attrValues config.treefmt.build.programs;
            extraPackages = with pkgs; [
              pkgs-unstable.cargo-audit
              cargo-machete
              cargo-shear
              zizmor
            ];
          };

          run-check = flake-utils.lib.mkApp {
            drv = pkgs.writeShellScriptBin "run-check" ''
              set -e
              check=$1
              if [ -z "$check" ]; then
                nix flake show --json 2>/dev/null | \
                  jq -r '.checks."${system}" | to_entries | .[].key' | \
                  xargs -I '{}' nix build ".#checks."${system}".{}"
              else
              	nix build ".#checks."${system}".$check"
              fi
            '';
          };
          run-audit = flake-utils.lib.mkApp {
            drv = pkgs.writeShellApplication {
              name = "audit";
              runtimeInputs = [
                pkgs.cargo
                pkgs-unstable.cargo-audit
              ];
              text = ''
                cargo audit
              '';
            };
          };
        in
        {
          treefmt = {
            inherit (config.flake-root) projectRootFile;

            settings.global.excludes = [
              "**/*.id"
              "**/.cargo-ok"
              "**/.gitignore"
              ".editorconfig"
              ".gitattributes"
              "LICENSE"
              "target/*"
            ];

            programs.shfmt.enable = true;
            settings.formatter.shfmt.includes = [
              "*.sh"
            ];

            programs.yamlfmt.enable = true;
            settings.formatter.yamlfmt.includes = [
              ".github/labeler.yml"
              ".github/workflows/*.yaml"
            ];
            settings.formatter.yamlfmt.settings = {
              formatter.type = "basic";
              formatter.max_line_length = 120;
              formatter.trim_trailing_whitespace = true;
              formatter.scan_folded_as_literal = true;
              formatter.include_document_start = true;
            };

            programs.prettier.enable = true;
            settings.formatter.prettier.includes = [
              "*.md"
              "*.json"
            ];
            settings.formatter.prettier.excludes = [
              "*.yml"
              "*.yaml"
            ];
            programs.rustfmt.enable = true;
            programs.nixfmt.enable = true;
            programs.taplo.enable = true;

            settings.formatter.rustfmt = {
              command = "${rustfmtWrapper}/bin/rustfmt";
            };
          };

          checks = { inherit (hoprApiPackages) clippy; };

          apps = {
            check = run-check;
            audit = run-audit;
          };

          packages = hoprApiPackages // {
            inherit pre-commit-check;
          };

          devShells.default = devShell;
          devShells.ci = ciShell;

          formatter = config.treefmt.build.wrapper;
        };
      # platforms which are supported as build environments
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
    };
}
