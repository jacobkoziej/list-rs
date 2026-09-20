{
  perSystem =
    {
      lib,
      pkgs,
      ...
    }:

    let
      inherit (lib) getExe;

      rust = pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;

    in
    {
      devShells.default = pkgs.mkShellNoCC (
        let
          pre-commit-bin = getExe pkgs.pre-commit;

        in
        {
          packages = with pkgs; [
            black
            rust
            commitlint-rs
            mdformat
            pre-commit
            statix
            toml-sort
            treefmt
            yamlfmt
            yamllint
          ];

          shellHook = ''
            ${pre-commit-bin} install --allow-missing-config > /dev/null
          '';
        }
      );
    };
}
