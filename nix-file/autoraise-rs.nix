# ── autoraise-rs: Nix / nix-darwin / home-manager integration ────────────────
#
# FILE LAYOUT (add these to your dotfiles):
#
#   flake.nix                        ← your existing flake (changes marked ##)
#   modules/autoraise-rs.nix         ← drop-in home-manager module (NEW)
#   packages/autoraise-rs/           ← the Rust source tree lives here
#     Cargo.toml
#     Cargo.lock                     ← MUST be committed for buildRustPackage
#     build.rs
#     src/
#       main.rs  raiser.rs  ...
#
# QUICK START:
#   1. Copy the Rust source into packages/autoraise-rs/ inside your dotfiles
#   2. Run `cd packages/autoraise-rs && cargo build` once to generate Cargo.lock
#   3. git add packages/autoraise-rs/Cargo.lock  ← required by Nix sandbox
#   4. Add the module import shown below to your home.nix / darwin config
#   5. darwin-rebuild switch --flake .
#
# ─────────────────────────────────────────────────────────────────────────────


# ══════════════════════════════════════════════════════════════════════════════
# 1.  packages/autoraise-rs/default.nix
#     Nix derivation that builds the binary — usable standalone or from a flake
# ══════════════════════════════════════════════════════════════════════════════
#
# { pkgs ? import <nixpkgs> { } }:
#
# pkgs.rustPlatform.buildRustPackage rec {
#   pname   = "autoraise-rs";
#   version = "1.0.0";
#
#   # Pull source from the same directory as this file.
#   # When used inside a flake, replace with:
#   #   src = self;   (or wherever you keep the Rust tree)
#   src = pkgs.lib.cleanSource ./.;
#
#   # Nix builds in a sandbox with no network — Cargo.lock must be committed.
#   cargoLock.lockFile = ./Cargo.lock;
#
#   # Link macOS frameworks that the Rust code calls via FFI.
#   # These are the same frameworks AutoRaise links against.
#   buildInputs = with pkgs.darwin.apple_sdk.frameworks; [
#     ApplicationServices
#     AppKit
#     Foundation
#     Carbon
#     CoreGraphics
#   ];
#
#   # Strip the binary in release mode (already set in Cargo.toml profile,
#   # but harmless to repeat here).
#   doCheck = false;   # no tests yet; skip cargo test
#
#   meta = {
#     description = "Focus-follows-mouse & auto-raise for macOS (AeroSpace-aware)";
#     platforms   = pkgs.lib.platforms.darwin;
#     mainProgram = "autoraise-rs";
#   };
# }


# ══════════════════════════════════════════════════════════════════════════════
# 2.  flake.nix  (only the CHANGED / ADDED lines — merge into your existing file)
# ══════════════════════════════════════════════════════════════════════════════
#
# {
#   description = "My nix-darwin + home-manager config";
#
#   inputs = {
#     nixpkgs.url       = "github:nixos/nixpkgs/nixpkgs-unstable";
#     nix-darwin.url    = "github:LnL7/nix-darwin";
#     nix-darwin.inputs.nixpkgs.follows = "nixpkgs";
#     home-manager.url  = "github:nix-community/home-manager";
#     home-manager.inputs.nixpkgs.follows = "nixpkgs";
#   };
#
#   outputs = { self, nixpkgs, nix-darwin, home-manager, ... }:
#   let
#     system = "aarch64-darwin";   # or "x86_64-darwin" for Intel
#     pkgs   = nixpkgs.legacyPackages.${system};
#
#     ## ── NEW: build the package once, reference it everywhere ──────────────
#     autoraise-rs = pkgs.callPackage ./packages/autoraise-rs { };
#     ## ──────────────────────────────────────────────────────────────────────
#   in {
#     darwinConfigurations."your-hostname" = nix-darwin.lib.darwinSystem {
#       inherit system;
#       modules = [
#         ./darwin-configuration.nix
#         home-manager.darwinModules.home-manager
#         {
#           home-manager.useGlobalPkgs = true;
#           home-manager.useUserPackages = true;
#           home-manager.users."your-username" = { imports = [
#             ./modules/autoraise-rs.nix   ## ← NEW import
#             ## pass the built package in so the module can reference it
#             { _module.args.autoraise-rs-pkg = autoraise-rs; }
#           ]; };
#         }
#       ];
#     };
#
#     ## expose it so you can also run `nix build .#autoraise-rs` directly
#     packages.${system}.autoraise-rs = autoraise-rs;   ## NEW
#   };
# }


# ══════════════════════════════════════════════════════════════════════════════
# 3.  modules/autoraise-rs.nix  ← THE MAIN FILE TO ADD
#     Home-manager module: installs binary + config + launchd agent
# ══════════════════════════════════════════════════════════════════════════════

# Save this file as   modules/autoraise-rs.nix
# Then import it in your home.nix:
#   imports = [ ./modules/autoraise-rs.nix ];

{ config, pkgs, lib, autoraise-rs-pkg, ... }:

let
  cfg = config.services.autoraise-rs;
in {

  # ── Option declarations ────────────────────────────────────────────────────
  options.services.autoraise-rs = {

    enable = lib.mkEnableOption "autoraise-rs focus-follows-mouse service";

    package = lib.mkOption {
      type    = lib.types.package;
      default = autoraise-rs-pkg;
      description = "The autoraise-rs package to use.";
    };

    pollMillis = lib.mkOption {
      type    = lib.types.int;
      default = 50;
      description = "Mouse poll interval in milliseconds (min 20).";
    };

    delay = lib.mkOption {
      type    = lib.types.int;
      default = 1;
      description = ''
        Raise delay in poll ticks.
          0  = raising disabled
          1  = raise instantly on hover (no stop required)
          2+ = mouse must stop for delay × pollMillis ms before raising
      '';
    };

    requireMouseStop = lib.mkOption {
      type    = lib.types.bool;
      default = true;
      description = "Require mouse to stop moving before raising (delay > 1 enforces this automatically).";
    };

    # ── AeroSpace ─────────────────────────────────────────────────────────────
    aerospaceAware = lib.mkOption {
      type    = lib.types.bool;
      default = true;
      description = ''
        AeroSpace tiling WM integration.
        true  = only raise FLOATING windows; skip tiled ones
                (AeroSpace manages tiled focus via hjkl bindings)
        false = raise all windows regardless of layout
      '';
    };

    aerospaceRefreshCycles = lib.mkOption {
      type    = lib.types.int;
      default = 10;
      description = "Poll cycles between AeroSpace floating-window list refreshes.";
    };

    # ── Disable key ───────────────────────────────────────────────────────────
    disableKey = lib.mkOption {
      type    = lib.types.enum [ "control" "option" "disabled" ];
      default = "control";
      description = "Hold this key to temporarily disable auto-raise.";
    };

    # ── Ignore lists ──────────────────────────────────────────────────────────
    ignoreApps = lib.mkOption {
      type    = lib.types.listOf lib.types.str;
      default = [];
      example = [ "Finder" "Activity Monitor" ];
      description = "App names to never auto-raise (case-insensitive exact match).";
    };

    ignoreTitles = lib.mkOption {
      type    = lib.types.listOf lib.types.str;
      default = [];
      example = [ "Picture in Picture" "Quick Look" ];
      description = "Window title substrings to never auto-raise.";
    };

    logFile = lib.mkOption {
      type    = lib.types.str;
      default = "/tmp/autoraise-rs.log";
      description = "Path for stdout log from the launchd agent.";
    };

  };

  # ── Implementation ─────────────────────────────────────────────────────────
  config = lib.mkIf cfg.enable {

    # 1. Install the binary into the user profile
    home.packages = [ cfg.package ];

    # 2. Write ~/.config/autoraise-rs/config.toml
    #    home-manager manages this file declaratively — editing it manually
    #    will be overwritten on next `darwin-rebuild switch`.
    xdg.configFile."autoraise-rs/config.toml".text = ''
      # autoraise-rs configuration
      # Managed by home-manager — edit via modules/autoraise-rs.nix options

      poll_millis             = ${toString cfg.pollMillis}
      delay                   = ${toString cfg.delay}
      require_mouse_stop      = ${lib.boolToString cfg.requireMouseStop}
      aerospace_aware         = ${lib.boolToString cfg.aerospaceAware}
      aerospace_refresh_cycles = ${toString cfg.aerospaceRefreshCycles}
      disable_key             = "${cfg.disableKey}"
      ignore_apps             = [${lib.concatMapStringsSep ", " (a: ''"${a}"'') cfg.ignoreApps}]
      ignore_titles           = [${lib.concatMapStringsSep ", " (t: ''"${t}"'') cfg.ignoreTitles}]
    '';

    # 3. Register the launchd user agent
    #    home-manager writes the plist to ~/Library/LaunchAgents/ and
    #    calls launchctl load/unload automatically on darwin-rebuild switch.
    launchd.agents.autoraise-rs = {
      enable = true;

      config = {
        # The binary path comes from the Nix store — always the right version.
        ProgramArguments = [
          "${cfg.package}/bin/autoraise-rs"
          # Config file is written by xdg.configFile above.
          # No extra args needed; the binary reads the file automatically.
        ];

        # Start at login, keep alive if it crashes.
        RunAtLoad  = true;
        KeepAlive  = true;

        # Nice value: run at low priority so it never competes with your work.
        Nice = 10;

        # Logs — tail with:  tail -f /tmp/autoraise-rs.log
        StandardOutPath   = cfg.logFile;
        StandardErrorPath = cfg.logFile;

        # ProcessType Background = low scheduling priority on macOS
        ProcessType = "Background";

        # Accessibility APIs need the main run loop — this is a foreground
        # agent (not a daemon), which is correct for a user-session tool.
        # LaunchOnlyOnce = false  (default; KeepAlive handles restarts)
      };
    };

  };
}


# ══════════════════════════════════════════════════════════════════════════════
# 4.  home.nix  — how to USE the module (add these lines to your existing file)
# ══════════════════════════════════════════════════════════════════════════════
#
# { config, pkgs, ... }:
# {
#   imports = [
#     ./modules/autoraise-rs.nix   # ← add this
#   ];
#
#   # ── autoraise-rs ──────────────────────────────────────────────────────────
#   services.autoraise-rs = {
#     enable      = true;
#
#     pollMillis  = 50;
#     delay       = 1;         # instant raise on hover
#     disableKey  = "control"; # hold ctrl to temporarily stop raising
#
#     # AeroSpace: only raise floating windows, skip tiled ones
#     aerospaceAware         = true;
#     aerospaceRefreshCycles = 10;
#
#     # Apps you NEVER want auto-raised
#     ignoreApps = [
#       "Finder"
#       "Activity Monitor"
#       "System Preferences"
#       "System Settings"
#     ];
#
#     # Window titles to skip (substring match)
#     ignoreTitles = [
#       "Picture in Picture"
#     ];
#   };
# }


# ══════════════════════════════════════════════════════════════════════════════
# 5.  After editing — apply the change
# ══════════════════════════════════════════════════════════════════════════════
#
#   darwin-rebuild switch --flake ~/.config/nix-darwin
#
# On FIRST install, macOS will prompt for Accessibility permission:
#   System Settings → Privacy & Security → Accessibility
#   The binary path will be something like:
#     /nix/store/xxxx-autoraise-rs-1.0.0/bin/autoraise-rs
#   Add that path and enable it.
#
# Because the Nix store path changes when the binary changes, you may need to
# re-grant permission after rebuilding.  To avoid this, symlink the binary:
#
#   # In your darwin-configuration.nix (system level, not home-manager):
#   environment.etc."autoraise-rs" = {
#     source = "${autoraise-rs}/bin/autoraise-rs";
#   };
#   # Then grant /etc/autoraise-rs in Accessibility — path never changes.
#
# Or use a stable wrapper path via home.file:
#
#   home.file.".local/bin/autoraise-rs" = {
#     source = "${cfg.package}/bin/autoraise-rs";
#     executable = true;
#   };
#   # Grant ~/.local/bin/autoraise-rs in Accessibility — survives rebuilds.
#
# ══════════════════════════════════════════════════════════════════════════════
