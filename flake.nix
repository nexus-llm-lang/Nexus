{
  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/*";
    flake-utils.url = "github:numtide/flake-utils";

    gitignore = {
      url = "github:hercules-ci/gitignore.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
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
      gitignore,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        rustBin = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "wasm32-wasip1" ];
        };

        rustPackages = [
          rustBin
          pkgs.rust-bin.stable.latest.rust-analyzer
        ];

        treeSitterNexus = pkgs.tree-sitter.buildGrammar {
          language = "nexus";
          version = "0.1.0";
          src = ./tree-sitter-nexus;
        };

        tsDeps = [ pkgs.nodejs pkgs.tree-sitter ];

        formatter = pkgs.nixfmt-tree;

        devShells.default = pkgs.mkShellNoCC {
          inputsFrom = [ treeSitterNexus ];
          packages = rustPackages ++ tsDeps ++ [
            pkgs.actionlint
            pkgs.nil
            formatter
          ];
        };
      in
      {
        packages = {
          tree-sitter-nexus = treeSitterNexus;
        };
        legacyPackages = pkgs;
        inherit formatter devShells;
      }
    );
}
