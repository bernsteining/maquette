{
  description = "maquette — headless CPU 3D renderer (STL/OBJ/PLY, glTF PBR, OpenSCAD) CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;

        # ── The `maquette` CLI, from the prebuilt dist release ────────────────
        # Building from source under Nix is impractical: the Manifold CSG kernel
        # is git-cloned by a build script at build time, which the Nix sandbox
        # (no network) forbids. So we wrap the release binary and patch it for
        # NixOS instead. Bump `version` to the released tag, then run
        # `nix build .#maquette` — Nix prints the real hash to paste in below.
        version = "0.1.4";
        asset = {
          "x86_64-linux" = "maquette-cli-x86_64-unknown-linux-gnu.tar.xz";
          "x86_64-darwin" = "maquette-cli-x86_64-apple-darwin.tar.xz";
          "aarch64-darwin" = "maquette-cli-aarch64-apple-darwin.tar.xz";
        }.${system} or null;
        hash = {
          "x86_64-linux" = lib.fakeHash;
          "x86_64-darwin" = lib.fakeHash;
          "aarch64-darwin" = lib.fakeHash;
        }.${system} or lib.fakeHash;

        maquette = pkgs.stdenv.mkDerivation {
          pname = "maquette";
          inherit version;
          src = pkgs.fetchurl {
            url = "https://github.com/bernsteining/maquette/releases/download/v${version}/${asset}";
            inherit hash;
          };
          sourceRoot = ".";
          nativeBuildInputs = lib.optionals pkgs.stdenv.isLinux [ pkgs.autoPatchelfHook ];
          buildInputs = lib.optionals pkgs.stdenv.isLinux [ pkgs.stdenv.cc.cc.lib ];
          installPhase = ''
            runHook preInstall
            install -Dm755 "$(find . -type f -name maquette | head -1)" "$out/bin/maquette"
            runHook postInstall
          '';
          meta = {
            description = "Headless CPU 3D renderer (STL/OBJ/PLY, glTF PBR, OpenSCAD) → PNG/SVG";
            homepage = "https://github.com/bernsteining/maquette";
            license = lib.licenses.mit;
            mainProgram = "maquette";
            platforms = [ "x86_64-linux" "x86_64-darwin" "aarch64-darwin" ];
          };
        };
      in
      {
        packages = lib.optionalAttrs (asset != null) {
          default = maquette;
          maquette = maquette;
        };

        apps = lib.optionalAttrs (asset != null) {
          default = {
            type = "app";
            program = "${maquette}/bin/maquette";
          };
        };

        # `nix develop` — a shell to build maquette (CLI + wasm) from source.
        # Unlike `nix build`, this shell has network, so the Manifold clone works.
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustc
            cargo
            clippy
            cmake
            clang
            lld
            pkg-config
            python3
            binaryen
            git
          ];
          shellHook = ''
            echo "maquette dev shell — try:  cargo build --release -p maquette-cli"
          '';
        };
      });
}
