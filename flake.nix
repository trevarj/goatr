{
  description = "Goatr direct Rust transport and source-pinned Android bindings";

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
      ffiIdentity = builtins.fromJSON (builtins.readFile ./third_party/iroh-ffi/source.json);
      sharedProvenance =
        lock:
        map
          (p: {
            inherit (p)
              name
              version
              source
              checksum
              ;
          })
          (
            builtins.filter (
              p:
              builtins.elem p.name [
                "iroh"
                "iroh-base"
                "iroh-relay"
                "rustls"
              ]
            ) (builtins.fromTOML (builtins.readFile lock)).package
          );
      companionProvenance = sharedProvenance ./companion/Cargo.lock;
      ffiProvenance = sharedProvenance ./third_party/iroh-ffi/Cargo.lock;
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
            pkgs.cargo-audit
            pkgs.pkg-config
            pkgs.sqlite
            pkgs.jq
            pkgs.unzip
          ];
          rust = pkgs.rust-bin.stable.latest.default;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
          ffiSource = pkgs.fetchzip {
            url = "${ffiIdentity.repository}/archive/${ffiIdentity.revision}.tar.gz";
            hash = ffiIdentity.narHash;
          };
          advisoryDb = pkgs.fetchzip {
            url = "${ffiIdentity.advisoryDatabase.repository}/archive/${ffiIdentity.advisoryDatabase.revision}.tar.gz";
            hash = ffiIdentity.advisoryDatabase.narHash;
          };
          patchedSource =
            assert builtins.hashFile "sha256" ./third_party/iroh-ffi/Cargo.lock == ffiIdentity.lockSha256;
            assert
              builtins.hashFile "sha256" ./third_party/iroh-ffi/patches/0001-secure-iroh-1.3.patch
              == ffiIdentity.patchSha256;
            assert
              (builtins.fromJSON (builtins.readFile ./third_party/iroh-ffi/dependencies.json)).lockSha256
              == ffiIdentity.lockSha256;
            pkgs.runCommand "goatr-iroh-ffi-source" { nativeBuildInputs = [ pkgs.patch ]; } ''
              cp -R ${ffiSource} "$out"
              chmod -R u+w "$out"
              patch --directory="$out" -p1 < ${./third_party/iroh-ffi/patches/0001-secure-iroh-1.3.patch}
              cp ${./third_party/iroh-ffi/Cargo.lock} "$out/Cargo.lock"
              cp ${./third_party/iroh-ffi/source.json} "$out/goatr-source.json"
              cp ${./third_party/iroh-ffi/dependencies.json} "$out/goatr-dependencies.json"
              cp ${./third_party/iroh-ffi/NOTICE} "$out/GOATR-NOTICE"
              cp ${./LICENSE} "$out/LICENSE-GOATR"
            '';
          companionDeps = rustPlatform.importCargoLock { lockFile = ./companion/Cargo.lock; };
          ffiDeps = rustPlatform.importCargoLock { lockFile = ./third_party/iroh-ffi/Cargo.lock; };
          checkCoreSource =
            deps:
            lib.concatMapStringsSep "\n"
              (name: ''
                jq -e --arg revision "${ffiIdentity.coreRevision}" \
                  '.git.sha1 == $revision' ${deps}/${name}-1.3.0/.cargo_vcs_info.json > /dev/null
              '')
              [
                "iroh"
                "iroh-base"
                "iroh-relay"
              ];
          installLicenses = deps: ''
            mkdir -p "$out/share/licenses/dependencies"
            for crate in ${deps}/*; do
              [ -d "$crate" ] || continue
              name=$(basename "$crate")
              while IFS= read -r -d "" notice; do
                relative="''${notice#"$crate"/}"
                destination="$out/share/licenses/dependencies/$name/$relative"
                mkdir -p "$(dirname "$destination")"
                cp "$notice" "$destination"
              done < <(find -L "$crate" -type f \
                \( -iname '*license*' -o -iname '*notice*' -o -iname 'copying*' -o -iname '*copyright*' \) -print0)
            done
          '';
          goatrManifest = pkgs.writeText "goatr-manifest.json" (
            builtins.toJSON {
              schema = 1;
              kind = "direct-rust";
              lockSha256 = builtins.hashFile "sha256" ./companion/Cargo.lock;
              coreRevision = ffiIdentity.coreRevision;
              sharedProvenance = companionProvenance;
              nixpkgsRevision = nixpkgs.rev;
              inherit (ffiIdentity) advisoryDatabase advisoryExclusions;
              systemSQLite = {
                version = pkgs.sqlite.version;
                source = pkgs.sqlite.src.url;
                hash = pkgs.sqlite.src.outputHash;
                use = "System SQLite selected for rusqlite storage; no bundled SQLite";
              };
            }
          );
          goatr =
            assert
              (builtins.fromJSON (builtins.readFile ./companion/dependencies.json)).lockSha256
              == builtins.hashFile "sha256" ./companion/Cargo.lock;
            rustPlatform.buildRustPackage {
              pname = "goatr";
              version = "0.1.0";
              src = lib.cleanSourceWith {
                src = ./companion;
                filter = path: type: lib.cleanSourceFilter path type && baseNameOf path != "target";
              };
              cargoDeps = companionDeps;
              nativeBuildInputs = [
                pkgs.pkg-config
                pkgs.jq
              ];
              buildInputs = [ pkgs.sqlite ];
              postPatch = checkCoreSource companionDeps;
              doCheck = false;
              postInstall = ''
                mkdir -p "$out/share/goatr" "$out/share/licenses/goatr"
                cp ${goatrManifest} "$out/share/goatr/manifest.json"
                cp ${./companion/Cargo.lock} ${./companion/dependencies.json} "$out/share/goatr/"
                cp ${./LICENSE} ${./NOTICE} "$out/share/licenses/goatr/"
                ${installLicenses companionDeps}
              '';
              meta = {
                description = "Goatr Linux desktop companion";
                license = lib.licenses.gpl3Plus;
                mainProgram = "goatr";
              };
            };
          android = lib.optionalAttrs (system == "x86_64-linux") (
            let
              androidPkgs = import nixpkgs {
                inherit system;
                config = {
                  # Explicit project consent, never global SDK settings.
                  android_sdk.accept_license = true;
                  allowUnfreePredicate = pkg: (pkg.meta.homepage or "") == "https://developer.android.com/tools";
                };
              };
              gradle = androidPkgs.gradle-packages.mkGradle {
                version = "9.8.0";
                hash = "sha256-uv1c6c+uoPvM/chDmhrEL71M2cidyamIIo2KJjmljmw=";
                defaultJava = androidPkgs.jdk21;
              };
              androidRust = rust.override {
                targets = [
                  "aarch64-linux-android"
                  "x86_64-linux-android"
                ];
              };
              androidRustPlatform = pkgs.makeRustPlatform {
                cargo = androidRust;
                rustc = androidRust;
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
              sdk = androidPkgs.androidenv.composeAndroidPackages sdkArgs;
              sdkRoot = "${sdk.androidsdk}/libexec/android-sdk";
              ndkRoot = "${sdkRoot}/ndk/28.2.13676358";
              irohAndroid = androidRustPlatform.buildRustPackage {
                pname = "goatr-iroh-android";
                version = "1.3.0";
                src = patchedSource;
                cargoDeps = ffiDeps;
                nativeBuildInputs = [
                  pkgs.cargo-ndk
                  pkgs.jq
                ];
                ANDROID_NDK_HOME = ndkRoot;
                ANDROID_NDK_ROOT = ndkRoot;
                auditable = false;
                dontStrip = true;
                doCheck = false;
                postPatch = checkCoreSource ffiDeps;
                buildPhase = ''
                  runHook preBuild
                  cargo build --frozen --release --bin uniffi-bindgen
                  CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384" \
                  CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384" \
                    cargo ndk --platform 26 --target arm64-v8a --target x86_64 \
                      --output-dir "$TMPDIR/goatr-jni" build --frozen --release --lib
                  mkdir -p generated-kotlin
                  target/release/uniffi-bindgen generate \
                    --library "$TMPDIR/goatr-jni/arm64-v8a/libiroh_ffi.so" \
                    --language kotlin --config uniffi.toml --out-dir generated-kotlin
                  runHook postBuild
                '';
                installPhase = ''
                  runHook preInstall
                  mkdir -p "$out/kotlin" "$out/android/computer/iroh" "$out/share/licenses/iroh-ffi"
                  # cargo-ndk also copies transitive cdylibs; none is an intended JNI input.
                  for abi in arm64-v8a x86_64; do
                    install -D -m 755 "$TMPDIR/goatr-jni/$abi/libiroh_ffi.so" \
                      "$out/jniLibs/$abi/libiroh_ffi.so"
                  done
                  cp -R generated-kotlin/. "$out/kotlin/"
                  cp kotlin/android/src/main/kotlin/computer/iroh/IrohAndroid.kt "$out/android/computer/iroh/"
                  cp Cargo.lock goatr-dependencies.json "$out/"
                  cp LICENSE-MIT LICENSE-APACHE LICENSE-GOATR GOATR-NOTICE "$out/share/licenses/iroh-ffi/"
                  ${installLicenses ffiDeps}
                  artifacts='{}'
                  pushd "$out"
                  while IFS= read -r -d "" artifact; do
                    hash=$(sha256sum "$artifact")
                    hash="''${hash%% *}"
                    artifacts=$(jq --arg path "$artifact" --arg hash "$hash" '. + {($path): $hash}' <<< "$artifacts")
                  done < <(find kotlin android jniLibs -type f -print0)
                  jq -n --slurpfile identity ${./third_party/iroh-ffi/source.json} \
                    --argjson provenance '${builtins.toJSON ffiProvenance}' \
                    --argjson artifacts "$artifacts" \
                    '{schema: 1, identity: $identity[0], abis: ["arm64-v8a", "x86_64"], sharedProvenance: $provenance, artifacts: $artifacts}' \
                    > manifest.json
                  popd
                  runHook postInstall
                '';
                meta.license = with lib.licenses; [
                  mit
                  asl20
                  gpl3Plus
                ];
              };
              mkAndroidShell =
                selectedSdk:
                let
                  root = "${selectedSdk.androidsdk}/libexec/android-sdk";
                in
                pkgs.mkShell {
                  packages = tools ++ [
                    androidRust
                    androidPkgs.jdk21
                    gradle
                    selectedSdk.androidsdk
                  ];
                  buildInputs = [ pkgs.sqlite ];
                  JAVA_HOME = "${androidPkgs.jdk21}";
                  ANDROID_HOME = root;
                  ANDROID_SDK_ROOT = root;
                  ANDROID_NDK_ROOT = "${root}/ndk/28.2.13676358";
                  GOATR_IROH_ANDROID = "${irohAndroid}";
                  GOATR_IROH_FFI_SOURCE = "${patchedSource}";
                  GOATR_ADVISORY_DB = "${advisoryDb}";
                  GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${root}/build-tools/36.0.0/aapt2";
                };
            in
            {
              inherit irohAndroid;
              shells = {
                android = mkAndroidShell sdk;
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
              };
            }
          );
        in
        {
          inherit
            pkgs
            tools
            rust
            goatr
            patchedSource
            android
            ;
          coreShell = pkgs.mkShell {
            packages = tools ++ [ rust ];
            buildInputs = [ pkgs.sqlite ];
            GOATR_ADVISORY_DB = "${advisoryDb}";
          };
        }
      );
    in
    assert companionProvenance == ffiProvenance;
    assert builtins.all (
      p: if p.name == "rustls" then lib.versionAtLeast p.version "0.23.45" else p.version == "1.3.0"
    ) companionProvenance;
    {
      packages = lib.mapAttrs (
        system: env:
        {
          inherit (env) goatr;
          default = env.goatr;
          iroh-ffi-source = env.patchedSource;
        }
        // lib.optionalAttrs (system == "x86_64-linux") { iroh-android = env.android.irohAndroid; }
      ) environments;
      apps = lib.mapAttrs (_: env: {
        goatr = {
          type = "app";
          program = "${env.goatr}/bin/goatr";
        };
      }) environments;
      devShells = lib.mapAttrs (
        system: env:
        { default = env.coreShell; } // lib.optionalAttrs (system == "x86_64-linux") env.android.shells
      ) environments;
    };
}
