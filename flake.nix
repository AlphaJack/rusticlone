{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      inherit (nixpkgs) lib;
      package = pkgs: pkgs.callPackage ./packaging/nix/package.nix { };
      systems = lib.filter (
        system: lib.meta.availableOn { inherit system; } (package nixpkgs.legacyPackages.${system})
      ) lib.systems.flakeExposed;
    in
    {
      packages = lib.genAttrs systems (system: {
        default = package nixpkgs.legacyPackages.${system};
      });
      overlays.default = final: _: { ${(lib.importTOML ./Cargo.toml).package.name} = package final; };
      nixosModules.default = ./packaging/nix/nixos.nix;
      darwinModules.default = ./packaging/nix/darwin.nix;
      homeModules.default = ./packaging/nix/home.nix;
      checks = lib.genAttrs [ "x86_64-linux" "aarch64-linux" ] (
        system: import ./packaging/nix/test.nix { pkgs = nixpkgs.legacyPackages.${system}; }
      );
    };
}
