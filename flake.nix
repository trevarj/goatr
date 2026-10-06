{
  description = "Goatr developer foundation (native implementation is phase 1A)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/767b0d3ec98a143ad9ed7dfc0d5553510ac27133";
    rust-overlay = {
      url = "github:oxalica/rust-overlay/e60029353d0c48d216bc4b065168ccd4079c166f";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      environments = lib.genAttrs systems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          tools = [
            pkgs.git
            pkgs.gnumake
            pkgs.nixfmt
            pkgs.cargo-ndk
          ];
          rust = pkgs.rust-bin.stable.latest.default;
        in
        {
          inherit pkgs tools rust;
          coreShell = pkgs.mkShell {
            packages = tools ++ [ rust ];
          };
        }
      );
    in
    {
      devShells = lib.mapAttrs (
        system: env:
        {
          default = env.coreShell;
        }
        // lib.optionalAttrs (system == "x86_64-linux") (
          let
            androidPkgs = import nixpkgs {
              inherit system;
              config = {
                # Explicit project consent; selecting this shell opts into the SDK.
                android_sdk.accept_license = true;
                allowUnfreePredicate = pkg: (pkg.meta.homepage or "") == "https://developer.android.com/tools";
              };
            };
            gradle = androidPkgs.gradle-packages.mkGradle {
              version = "9.8.0";
              hash = "sha256-uv1c6c+uoPvM/chDmhrEL71M2cidyamIIo2KJjmljmw=";
              defaultJava = androidPkgs.jdk21;
            };
            androidRust = env.rust.override {
              targets = [
                "aarch64-linux-android"
                "x86_64-linux-android"
              ];
            };
            sdkArgs = {
              platformVersions = [ "37.0" ];
              buildToolsVersions = [ "36.0.0" ];
              platformToolsVersion = "35.0.2";
              cmdLineToolsVersion = "21.0";
              toolsVersion = null;
              includeNDK = true;
              ndkVersions = [ "28.2.13676358" ];
              includeCmake = false;
              includeSources = false;
              includeSystemImages = false;
              includeEmulator = false;
              useGoogleAPIs = false;
              useGoogleTVAddOns = false;
              includeExtras = [ ];
            };
            mkAndroidShell =
              sdk:
              let
                sdkRoot = "${sdk.androidsdk}/libexec/android-sdk";
              in
              env.pkgs.mkShell {
                packages = env.tools ++ [
                  androidRust
                  androidPkgs.jdk21
                  gradle
                  sdk.androidsdk
                ];
                JAVA_HOME = "${androidPkgs.jdk21}";
                ANDROID_HOME = sdkRoot;
                ANDROID_SDK_ROOT = sdkRoot;
                ANDROID_NDK_ROOT = "${sdkRoot}/ndk/28.2.13676358";
                GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdkRoot}/build-tools/36.0.0/aapt2";
              };
          in
          {
            android = mkAndroidShell (androidPkgs.androidenv.composeAndroidPackages sdkArgs);
            emulator = mkAndroidShell (
              androidPkgs.androidenv.composeAndroidPackages (
                sdkArgs
                // {
                  platformVersions = [ "34" ];
                  includeEmulator = true;
                  emulatorVersion = "36.6.11";
                  includeSystemImages = true;
                  systemImageTypes = [ "default" ];
                  abiVersions = [ "x86_64" ];
                }
              )
            );
          }
        )
      ) environments;
    };
}
