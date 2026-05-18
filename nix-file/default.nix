# packages/autoraise-rs/default.nix
# Build the autoraise-rs binary inside the Nix sandbox.
#
# Usage:
#   nix-build          (classic Nix)
#   nix build .#autoraise-rs   (flake)
#   pkgs.callPackage ./packages/autoraise-rs { }   (from flake.nix)

{ pkgs ? import <nixpkgs> { }
, lib ? pkgs.lib
}:

pkgs.rustPlatform.buildRustPackage rec {
  pname   = "autoraise-rs";
  version = "1.0.0";

  # Source is the directory containing this file (Cargo.toml, src/, etc.)
  # lib.cleanSource strips .git, target/, *.log so Nix hash stays stable.
  src = lib.cleanSource ./.;

  # Nix sandbox has no network access — Cargo.lock must be committed.
  # `cargo vendor` is run automatically by buildRustPackage using the lock file.
  cargoLock.lockFile = ./Cargo.lock;

  # macOS framework dependencies (same set as our build.rs links against).
  buildInputs = with pkgs.darwin.apple_sdk.frameworks; [
    ApplicationServices
    AppKit
    Foundation
    Carbon
    CoreGraphics
  ];

  # Native build tools (Rust compiler etc. are provided by buildRustPackage).
  nativeBuildInputs = [ pkgs.libiconv ];

  # We link against macOS AXRuntime which is part of ApplicationServices.
  # Tell the linker where to find it (Nix sandbox layout).
  NIX_LDFLAGS = "-F${pkgs.darwin.apple_sdk.frameworks.ApplicationServices}/Library/Frameworks";

  # Skip `cargo test` — tests require a running macOS session with Accessibility.
  doCheck = false;

  # Release profile (lto, strip) already set in Cargo.toml.
  buildType = "release";

  meta = with lib; {
    description = "Focus-follows-mouse & auto-raise for macOS — AeroSpace-aware, written in Rust";
    homepage    = "https://github.com/you/autoraise-rs";
    license     = licenses.mit;
    platforms   = platforms.darwin;
    mainProgram = "autoraise-rs";
  };
}
