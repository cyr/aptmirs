{
  description = "Nix flake for aptmirs";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      package = cargoToml.package;
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSupportedSystem =
        f:
        lib.genAttrs supportedSystems (
          system:
          f {
            inherit system;
            pkgs = import nixpkgs { inherit system; };
          }
        );
    in
    {
      packages = forEachSupportedSystem (
        { pkgs, ... }:
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = package.name;
            version = package.version;
            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = with pkgs; [
              pkg-config
            ];

            buildInputs = with pkgs; [
              bzip2
              xz
            ];

            meta = with pkgs.lib; {
              description = package.description;
              homepage = package.repository;
              license = licenses.mit;
              mainProgram = package.name;
              platforms = platforms.unix;
            };
          };
        }
      );

      apps = forEachSupportedSystem (
        { system, ... }:
        {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/${package.name}";
          };
        }
      );

      devShells = forEachSupportedSystem (
        { pkgs, ... }:
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              nixpkgs-fmt
              pkg-config
              rust-analyzer
              rustc
              rustfmt
              bzip2
              xz
            ];
          };
        }
      );

      formatter = forEachSupportedSystem ({ pkgs, ... }: pkgs.nixfmt-tree);
    };
}
