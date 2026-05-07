{
  inputs = {
    nixpkgs.url = "github:/NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        runtimeDeps = with pkgs; [
          wasmtime
          wabt
          binaryen
        ];

        formatter = pkgs.nixfmt-tree;

        devShells.default = pkgs.mkShellNoCC {
          packages =
            runtimeDeps
            ++ [
              pkgs.wasm-tools
              pkgs.actionlint
              pkgs.nil
              formatter
            ];
        };
      in
      {
        legacyPackages = pkgs;
        inherit formatter devShells;
      }
    );
}
