{
  description = "Development and build environment for wrx.sh monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-25.11";
    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    prisma-utils = {
      url = "github:VanCoding/nix-prisma-utils";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };

    crane.url = "github:ipetkov/crane/refs/tags/v0.22.0";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      prisma-utils,
      crane,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Use crane with the latest stable toolchain for reproducible builds
        craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.default);

        # Prisma engines for NixOS (no upstream binaries). On non-NixOS, prisma will download engines.
        prisma =
          (prisma-utils.lib.prisma-factory {
            inherit pkgs;
            prisma-fmt-hash = "sha256-CRxh1NftBvR6mRl9DugLkCvOSqQ3jdcpPw3HUXpJi6I=";
            query-engine-hash = "sha256-64AcHjA07mSFQJ4ZbazorXCOzlK6TIlYXPABj/Wu4Ck=";
            libquery-engine-hash = "sha256-vJcVAP+RWOIh7fgeHSL8Zq33IgrTLZlOggku+QnKT2E=";
            schema-engine-hash = "sha256-0cCQsDinecDuvVRxdXEMo+yZWbJBsXpTeZl3i3yI4Cc=";
          }).fromNpmLock
            ./package-lock.json;

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "clippy"
            "rust-analyzer"
          ];
        };

        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
      in
      rec {
        # Build the Rust API with crane (workspace root). The bin is "api" crate.
        packages.api =
          let
            commonArgs = {
              pname = (craneLib.crateNameFromCargoToml { cargoToml = ./api/Cargo.toml; }).pname;
              version = (craneLib.crateNameFromCargoToml { cargoToml = ./api/Cargo.toml; }).version;
              src = nixpkgs.lib.cleanSourceWith {
                src = ./.;
                filter =
                  path: type:
                  ((path: _type: builtins.match ".*api/.*html$" path != null) path type)
                  || (craneLib.filterCargoSources path type);
                name = "source";
              };
              strictDeps = true;
              doCheck = false; # TODO: enable when tests are green
              dontStrip = true; # preserve backtrace
              buildInputs = with pkgs; [
                openssl
                libxml2
              ];
              nativeBuildInputs = with pkgs; [
                pkg-config
                libpq
                clang # rust-bindgen
              ];
              inherit LIBCLANG_PATH;
            };
          in
          craneLib.buildPackage (
            commonArgs
            // {
              cargoArtifacts = craneLib.buildDepsOnly commonArgs;
            }
          );

        packages.dockerApi = pkgs.dockerTools.buildLayeredImage {
          name = "ghcr.io/wonrax/wrx-sh-api";
          tag = "latest";
          contents = with pkgs; [ cacert ];
          config = {
            Env = [ "RUST_LOG=info" ];
            Cmd = [ "${packages.api}/bin/api" ];
          };
        };

        packages.www = pkgs.buildNpmPackage {
          name = "www";
          src = ./.;

          npmDeps = pkgs.importNpmLock { npmRoot = ./.; };
          npmConfigHook = pkgs.importNpmLock.npmConfigHook;
          npmWorkspace = ./.;

          env.PUBLIC_GIT_REV = builtins.substring 0 7 (self.rev or self.dirtyRev or "unknown");

          buildPhase = ''
            npx turbo build --filter=web
          '';

          installPhase = ''
            mkdir -p $out
            cp -r web/dist/* $out/
          '';
        };

        packages.dockerWww = pkgs.dockerTools.buildLayeredImage {
          name = "ghcr.io/wonrax/wrx-sh-www";
          tag = "latest";
          contents = with pkgs; [ busybox ];
          config = {
            Entrypoint = [
              "sh"
              "-c"
              "rm -rf /.mount/* && cp -r ${packages.www}/* /.mount"
            ];
          };
        };

        packages.schemaMigrator = pkgs.stdenv.mkDerivation {
          name = "schema-migrator";
          src = ./.;

          installPhase = ''
            mkdir -p $out
            cp -r prisma $out
          '';
        };

        packages.dockerSchemaMigrator = pkgs.dockerTools.buildLayeredImage {
          name = "ghcr.io/wonrax/wrx-sh-migrator";
          tag = "latest";
          contents = with pkgs; [ nodejs_22 ];
          config = {
            Cmd = [
              "${pkgs.lib.getExe pkgs.prisma}"
              "migrate"
              "deploy"
            ];
            WorkingDir = "${packages.schemaMigrator}";
          };
          # dockerTools image does not have /tmp by default, prisma needs this
          # https://discourse.nixos.org/t/dockertools-buildimage-and-user-writable-tmp/5397/9
          extraCommands = "mkdir -m 0777 tmp";
        };

        devShells.default =
          with pkgs;
          mkShell {
            nativeBuildInputs = [
              diesel-cli
              rustToolchain
            ];

            inputsFrom = [
              packages.api
              packages.www
              packages.schemaMigrator
            ];

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            inherit LIBCLANG_PATH;

            # Only needed on NixOS
            shellHook = if system == "x86_64-linux" then prisma.shellHook else "";
          };
      }
    );
}
