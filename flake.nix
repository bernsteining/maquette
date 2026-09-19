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

        # Manifold's CSG kernel and its Clipper2 dependency are normally cloned by
        # a build script at build time — forbidden in the Nix sandbox (no network).
        # Pre-fetch both as fixed-output derivations and hand them to the build via
        # MANIFOLD_SRC / CLIPPER2_SRC (see crates/vendored/manifold-csg-sys), which
        # skips the clones and forces a fully-disconnected cmake build.
        #
        # First build prints the real hashes to replace the lib.fakeHash placeholders
        # (this one, the two below, and the four git deps in cargoLock.outputHashes).
        manifoldSrc = pkgs.fetchFromGitHub {
          owner = "elalish";
          repo = "manifold";
          rev = "v3.5.2";
          hash = lib.fakeHash;
        };
        clipper2Src = pkgs.fetchFromGitHub {
          owner = "AngusJohnson";
          repo = "Clipper2";
          rev = "46f639177fe418f9689e8ddb74f08a870c71f5b4";
          hash = lib.fakeHash;
        };

        maquette = pkgs.rustPlatform.buildRustPackage {
          pname = "maquette";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "gltf-1.4.1" = lib.fakeHash;
              "gltf-derive-1.4.1" = lib.fakeHash;
              "gltf-json-1.4.1" = lib.fakeHash;
              "wasm-minimal-protocol-0.2.1" = lib.fakeHash;
            };
          };

          # Only the native CLI; skip the workspace tests (they read example assets).
          cargoBuildFlags = [ "--package" "maquette-cli" ];
          doCheck = false;

          nativeBuildInputs = [ pkgs.cmake pkgs.pkg-config ];

          # Hermetic Manifold build (no network) — see the note above.
          MANIFOLD_SRC = manifoldSrc;
          CLIPPER2_SRC = clipper2Src;

          meta = {
            description = "Headless CPU 3D renderer (STL/OBJ/PLY, glTF PBR, OpenSCAD) → PNG/SVG";
            homepage = "https://github.com/bernsteining/maquette";
            license = lib.licenses.mit;
            mainProgram = "maquette";
          };
        };
      in
      {
        packages.default = maquette;
        packages.maquette = maquette;

        apps.default = {
          type = "app";
          program = "${maquette}/bin/maquette";
        };

        # `nix develop` — a shell to hack on maquette (CLI + wasm) from source.
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
            echo "maquette dev shell — cargo build --release -p maquette-cli"
          '';
        };
      });
}
