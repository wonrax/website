{
  description = "Development environment for hhai.dev monorepo";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-24.11";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
    prisma-utils = {
      url = "github:VanCoding/nix-prisma-utils";
      inputs = {
        pkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      prisma-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Patch for NixOS only because prisma does not release binaries for
        # NixOS. On other systems, running the prisma CLI will automatically
        # download the correct binary.
        prisma =
          (prisma-utils.lib.prisma-factory {
            inherit pkgs;
            prisma-fmt-hash = "sha256-ggfTlnrRle8HorCCPHa23OO3YBQE1A3yPPAvkq4Ki8M=";
            query-engine-hash = "sha256-VuFWwhnNXlAPDrVM+BD9vj2tJdrSVLBofFLph5LBaR4=";
            libquery-engine-hash = "sha256-PeZ1cfNzzlVGy8y6mqpeXWj7KCPQmaW+5EzsVcX+XG0=";
            schema-engine-hash = "sha256-58Dw7bZGxQ9jeWU6yeBl+BZQagke1079cIAHvYL01Cg=";
          }).fromNpmLock
            ./package-lock.json;

      in
      {
        devShells.default =
          with pkgs;
          mkShell {
            buildInputs = [
              rust-bin.stable.latest.default
              nodejs_22
              # libpq required for sqlx
              postgresql_17
            ];

            shellHook = if system == "x86_64-linux" then prisma.shellHook else "";
          };
      }
    );
}
