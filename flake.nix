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

        prisma = prisma-utils.lib.prisma-factory {
          inherit pkgs;
          hash =
            if pkgs.stdenv.hostPlatform.isLinux then
              "sha256-c3ryuV+IG2iumFPOBdcEgF0waa+KGrn7Ken2CRuupwg="
            else
              "sha256-PBsKvHfrF8AuSbRr3gHGPpouEBtThd7rEMLNZmOd0Ts=";
          npmLock = ./package-lock.json;
        };

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

        # FIXME: bloated image due to node_modules inclusion, find a way to
        # slim it down
        packages.schemaMigrator =
          let
            prismaWrapped = pkgs.writeScriptBin "prisma" ''
              #!${pkgs.bash}/bin/sh
              # export every env in prisma.env to environment
              ${pkgs.lib.concatMapStringsSep "\n" (env: "export ${env}") (
                pkgs.lib.mapAttrsToList (n: v: "${n}='${v}'") prisma.env
              )}
              exec ./node_modules/.bin/prisma "$@"
            '';
          in
          pkgs.buildNpmPackage {
            name = "schema-migrator";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./prisma
                ./prisma.config.ts
                ./package.json
                ./package-lock.json
              ];
            };

            npmDeps = pkgs.importNpmLock { npmRoot = ./.; };
            npmConfigHook = pkgs.importNpmLock.npmConfigHook;
            npmWorkspace = ./.;
            dontNpmBuild = true;
            dontCheckForBrokenSymlinks = true;

            buildInputs = [ pkgs.openssl ];

            installPhase = ''
              mkdir -p $out
              cp -r prisma $out/
              cp -r prisma.config.ts $out/
              cp -r node_modules $out
              cp -r ${prismaWrapped}/bin $out/
            '';

            doInstallCheck = true;
            installCheckPhase = ''
              cd $out
              DATABASE_URL=hehe ${prismaWrapped}/bin/prisma validate
            '';
          };

        packages.dockerSchemaMigrator = pkgs.dockerTools.buildLayeredImage {
          name = "ghcr.io/wonrax/wrx-sh-migrator";
          tag = "latest";
          contents = [ packages.schemaMigrator ];
          config = {
            Env = (pkgs.lib.mapAttrsToList (n: v: "${n}=${v}") prisma.env);
            Cmd = [
              "prisma"
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
            env = prisma.env;

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
          };
      }
    );
}
