{
  description = "Development and build environment for wrx.sh monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-25.05";
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

    crane.url = "github:ipetkov/crane";
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
            prisma-fmt-hash = "sha256-ggfTlnrRle8HorCCPHa23OO3YBQE1A3yPPAvkq4Ki8M=";
            query-engine-hash = "sha256-VuFWwhnNXlAPDrVM+BD9vj2tJdrSVLBofFLph5LBaR4=";
            libquery-engine-hash = "sha256-PeZ1cfNzzlVGy8y6mqpeXWj7KCPQmaW+5EzsVcX+XG0=";
            schema-engine-hash = "sha256-58Dw7bZGxQ9jeWU6yeBl+BZQagke1079cIAHvYL01Cg=";
          }).fromNpmLock
            ./package-lock.json;

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "clippy"
            "rust-analyzer"
          ];
        };

        # Common deps for the Rust API crate
        buildInputs = with pkgs; [
          openssl
          libxml2
        ];
        nativeBuildInputs = with pkgs; [
          pkg-config
          libpq
          clang # rust-bindgen
        ];

        LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
      in
      rec {
        # Build the Rust API with crane (workspace root). The bin is "api" crate.
        packages.api =
          let
            commonArgs = {
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
              inherit buildInputs nativeBuildInputs;
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

          env.PUBLIC_GIT_REV = self.dirtyShortRev;

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

        devShells.default =
          with pkgs;
          mkShell {
            buildInputs = [
              nodejs_22
              rustToolchain
              rust-analyzer-unwrapped
              diesel-cli
            ]
            ++ buildInputs
            ++ nativeBuildInputs;

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            inherit LIBCLANG_PATH;

            # Only needed on NixOS
            shellHook = if system == "x86_64-linux" then prisma.shellHook else "";
          };
      }
    );
}
