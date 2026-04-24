{
  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/*";
    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        rustBin = pkgs.rust-bin.stable.latest.default.override {
          targets = [
            "wasm32-wasip1"
            "wasm32-wasip2"
          ];
        };

        rustPackages = [
          rustBin
          pkgs.rust-bin.stable.latest.rust-analyzer
        ];

        runtimeDeps = with pkgs; [
          wasmtime
          wabt
          binaryen
        ];

        formatter = pkgs.nixfmt-tree;

        devShells.default = pkgs.mkShellNoCC {
          packages =
            rustPackages
            ++ runtimeDeps
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
