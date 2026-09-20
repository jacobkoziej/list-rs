# SPDX-License-Identifier: MPL-2.0

{
  description = "Intrusive doubly linked list written in Rust";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs:

    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-darwin"
        "x86_64-linux"
      ];

      imports = [
        ./nix
      ];

      perSystem =
        {
          pkgs,
          system,
          ...
        }:

        {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;

            overlays = [
              inputs.rust-overlay.overlays.default
            ];
          };

          formatter = pkgs.nixfmt-rfc-style;
        };
    };
}
