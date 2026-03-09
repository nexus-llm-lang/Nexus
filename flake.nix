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
          targets = [
            "wasm32-wasip1"
            "wasm32-wasip2"
          ];
        };

        rustPackages = [
          rustBin
          pkgs.rust-bin.stable.latest.rust-analyzer
        ];

        tsNexus = pkgs.tree-sitter.buildGrammar {
          language = "nexus";
          version = "0.1.0";
          src = ./tree-sitter-nexus;
        };

        tsDeps = with pkgs; [
          nodejs
          tree-sitter
        ];
        runtimeDeps = with pkgs; [
          wasmtime
          wabt
          binaryen
          lld
        ];

        formatter = pkgs.nixfmt-tree;

        devShells.default = pkgs.mkShellNoCC {
          inputsFrom = [ tsNexus ];
          packages =
            rustPackages
            ++ tsDeps
            ++ runtimeDeps
            ++ [
              pkgs.actionlint
              pkgs.nil
              formatter
            ];
        };

        devShells.docs = pkgs.mkShellNoCC {
          inputsFrom = [ tsNexus ];
          packages =
            with pkgs.rubyPackages;
            [
              pkgs.ruby
              jekyll
              jekyll-theme-slate
              jekyll-seo-tag
              kramdown-parser-gfm
            ]
            ++ tsDeps;
        };
      in
      {
        packages.tree-sitter-nexus = tsNexus;
        legacyPackages = pkgs;
        inherit formatter devShells;
      }
    );
}
