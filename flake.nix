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
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
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
          hcRelease =
            {
              x86_64-linux = {
                target = "linux-x86_64";
                sha256 = "4adcfb423d52f24c5cc4a1e2cd2fcad1f62d28bb77876ac472c29082a3eaa424";
              };
              aarch64-linux = {
                target = "linux-aarch64";
                sha256 = "f95c34fd7177ed58807d74fe4ee78bb3dc73eb37f21c69496064000a21abcb10";
              };
              x86_64-darwin = {
                target = "macos-x86_64";
                sha256 = "2f95bedae2ca1bf9f9611d61066ef3ac211c62ca08282132d48e287b6da14d08";
              };
              aarch64-darwin = {
                target = "macos-aarch64";
                sha256 = "5338d900adadb6d273b9c8e97027247db3179ccffede10a1793358f16e08d683";
              };
            }
            .${system};
          # Official, unmodified release archive; upstream HTTPS manifest is not independently signed.
          hc = pkgs.stdenvNoCC.mkDerivation {
            pname = "herdr-core";
            version = "0.1.19";
            src = pkgs.fetchurl {
              url = "https://releases.herdr.dev/core/0.1.19/herdr-core-${hcRelease.target}.tar.gz";
              inherit (hcRelease) sha256;
            };
            dontUnpack = true;
            dontFixup = true;
            installPhase = ''
              tar -xzf "$src" herdr-core LICENSE THIRD-PARTY-NOTICES
              install -Dm755 herdr-core "$out/bin/hc"
              install -Dm644 LICENSE "$out/share/licenses/herdr-core/LICENSE"
              install -Dm644 THIRD-PARTY-NOTICES "$out/share/licenses/herdr-core/THIRD-PARTY-NOTICES"
            '';
            meta = {
              mainProgram = "hc";
              license = lib.licenses.asl20;
            };
          };
          # One locked derivation serves the product and every development shell.
          readGit = pkgs.git;
          tools = [
            readGit
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
          goatrManifest =
            pkgs.runCommand "goatr-manifest"
              {
                __structuredAttrs = true;
                nativeBuildInputs = [
                  pkgs.coreutils
                  pkgs.findutils
                  pkgs.gnutar
                  pkgs.xz
                  pkgs.jq
                ]
                ++ lib.optional pkgs.stdenv.hostPlatform.isLinux pkgs.patchelf;
                exportReferencesGraph.readGitClosure = [ readGit ];
                manifestBase = {
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
                  readGit = {
                    schema = 1;
                    system = system;
                    package = {
                      output = "${readGit}";
                      derivation = readGit.drvPath;
                      version = readGit.version;
                      source = {
                        url = readGit.src.url;
                        hash = readGit.src.outputHash;
                      };
                      patches = map toString readGit.patches;
                      configureFlags = readGit.configureFlags;
                      makeFlags = readGit.makeFlags;
                      licenses = map (license: license.spdxId or license.shortName) (lib.toList readGit.meta.license);
                    };
                    nixpkgsSource = "${nixpkgs.outPath}";
                    nixpkgsNarHash = (builtins.fromJSON (builtins.readFile ./flake.lock)).nodes.nixpkgs.locked.narHash;
                    recipe = "${nixpkgs.outPath}/pkgs/by-name/gi/git/package.nix";
                    executable = "${readGit}/bin/git";
                    execPath = "${readGit}/libexec/git-core";
                    systemConfig = "/etc/gitconfig";
                    policy = "direct immutable Git ELF only; approved-root reads; no writes, network or helper exec; no PATH/runtime-env override";
                  };
                };
              }
              ''
                out=''${outputs[out]}
                mkdir -p "$out/licenses/git" "$out/licenses/runtime"
                jq '.readGitClosure' "$NIX_ATTRS_JSON_FILE" > "$out/runtime-closure.json"
                jq -r '.readGitClosure[].path' "$NIX_ATTRS_JSON_FILE" > "$out/runtime-closure.store-paths"
                jq -r '.readGitClosure | map([.path,.narHash,.narSize,"",(.references|length)]+.references) | add | .[]' \
                  "$NIX_ATTRS_JSON_FILE" > "$out/runtime-closure.registration"
                jq '.manifestBase' "$NIX_ATTRS_JSON_FILE" > "$TMPDIR/manifest-base.json"
                available=false
                reason=unsupported_platform
                executable=${lib.escapeShellArg "${readGit}/bin/git"}
                interpreter=
                interpreterRequest=
                executableSha256=
                interpreterSha256=
                if ${if pkgs.stdenv.hostPlatform.isLinux then "true" else "false"}; then
                  reason=git_not_direct_elf
                  read -r byte0 byte1 byte2 byte3 < <(od -An -N4 -tx1 "$executable")
                  if [[ "$byte0 $byte1 $byte2 $byte3" == "7f 45 4c 46" \
                    && "$(realpath -- "$executable")" == "$executable" ]]; then
                    reason=interpreter_unproved
                    interpreterRequest=$(patchelf --print-interpreter "$executable" 2>/dev/null || true)
                    if [[ "$interpreterRequest" == /nix/store/* && -f "$interpreterRequest" ]]; then
                      interpreter=$(realpath -- "$interpreterRequest")
                      read -r executableSha256 _ < <(sha256sum -- "$executable")
                      read -r interpreterSha256 _ < <(sha256sum -- "$interpreter")
                      available=true
                      reason=qualified_direct_layout
                    fi
                  fi
                fi
                if ! tar -xOf ${readGit.src} ${lib.escapeShellArg "git-${readGit.version}/COPYING"} \
                  > "$out/licenses/git/COPYING"; then
                  available=false
                  reason=license_unavailable
                fi
                : > "$TMPDIR/runtime-notices.jsonl"
                while IFS= read -r member; do
                  while IFS= read -r -d "" notice; do
                    relative="''${notice#"$member"/}"
                    if [[ "$relative" == "$notice" ]]; then relative="''${notice##*/}"; fi
                    destination="$out/licenses/runtime/''${member##*/}/$relative"
                    mkdir -p "$(dirname -- "$destination")"
                    cp -- "$notice" "$destination"
                    read -r noticeHash _ < <(sha256sum -- "$notice")
                    jq -cn --arg member "$member" --arg path "$notice" --arg retained "$destination" \
                      --arg sha256 "$noticeHash" '{member:$member,path:$path,retained:$retained,sha256:$sha256}' \
                      >> "$TMPDIR/runtime-notices.jsonl"
                  done < <(find "$member" -type f \
                    \( -iname '*license*' -o -iname '*notice*' -o -iname 'copying*' -o -iname '*copyright*' \) -print0)
                done < "$out/runtime-closure.store-paths"
                read -r closureSha256 _ < <(sha256sum -- "$out/runtime-closure.store-paths")
                read -r registrationSha256 _ < <(sha256sum -- "$out/runtime-closure.registration")
                jq -s '.' "$TMPDIR/runtime-notices.jsonl" > "$TMPDIR/runtime-notices.json"
                jq --argjson available "$available" --arg reason "$reason" \
                  --arg executable "$executable" --arg executableSha256 "$executableSha256" \
                  --arg interpreterRequest "$interpreterRequest" --arg interpreter "$interpreter" \
                  --arg interpreterSha256 "$interpreterSha256" --arg root "$out" \
                  --arg closureSha256 "$closureSha256" --arg registrationSha256 "$registrationSha256" \
                  --slurpfile runtimeNotices "$TMPDIR/runtime-notices.json" \
                  '.readGit += {available:$available,reason:$reason,executable:$executable,
                    executableSha256:$executableSha256,interpreterRequest:$interpreterRequest,
                    interpreter:$interpreter,interpreterSha256:$interpreterSha256,
                    closureList:($root+"/runtime-closure.store-paths"),closureSha256:$closureSha256,
                    registration:($root+"/runtime-closure.registration"),registrationSha256:$registrationSha256,
                    closureGraph:($root+"/runtime-closure.json"),gitLicense:($root+"/licenses/git/COPYING"),
                    runtimeNotices:$runtimeNotices[0],
                    licenseProvenance:"Actual Git source COPYING and packaged runtime notices; exact Nixpkgs recipes/derivation and closure NAR graph retained separately from Cargo notices"}' \
                  "$TMPDIR/manifest-base.json" > "$out/manifest.json"
              '';
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
                pkgs.makeWrapper
              ];
              buildInputs = [ pkgs.sqlite ];
              GOATR_READ_GIT_MANIFEST = "${goatrManifest}/manifest.json";
              postPatch = checkCoreSource companionDeps;
              doCheck = false;
              postInstall = ''
                mkdir -p "$out/share/goatr" "$out/share/licenses/goatr"
                cp ${goatrManifest}/manifest.json "$out/share/goatr/manifest.json"
                ln -s ${goatrManifest} "$out/share/goatr/read-git-provenance"
                ln -s ${goatrManifest}/licenses "$out/share/licenses/goatr/read-git"
                cp ${./companion/Cargo.lock} ${./companion/dependencies.json} "$out/share/goatr/"
                cp ${./LICENSE} ${./NOTICE} "$out/share/licenses/goatr/"
                ${installLicenses companionDeps}
                wrapProgram "$out/bin/goatr" --set-default GOATR_HC "${hc}/bin/hc"
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
              zig = pkgs.stdenvNoCC.mkDerivation {
                pname = "zig";
                version = "0.17.0";
                src = pkgs.fetchurl {
                  url = "https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz";
                  sha256 = "1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026";
                };
                dontFixup = true;
                installPhase = ''
                  mkdir -p "$out/bin"
                  cp -r zig lib LICENSE "$out/"
                  ln -s "$out/zig" "$out/bin/zig"
                '';
                meta.license = lib.licenses.mit;
              };
              ghostty = pkgs.fetchzip {
                url = "https://codeload.github.com/ghostty-org/ghostty/tar.gz/0fa3d577a5614150ff8e2c152de5fb9d5c6d2dec";
                extension = "tar.gz";
                hash = "sha256-XHatB+PU7Os1/inapleBdUThGCAGhRnqT0o0EWpNa3s=";
              };
              # Reuse upstream's generated, hash-checked complete transitive package graph.
              zigPackages =
                (pkgs.callPackage "${ghostty}/deps/packages.nix" {
                  zig_0_17 = zig;
                }).overrideAttrs
                  (old: {
                    buildCommand = "export NIX_BUILD_CORES=2\n" + old.buildCommand;
                  });
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
                ndkVersions = [
                  "28.2.13676358"
                  "27.1.12297006"
                ];
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
              # Keep the unchanged iroh producer independent of the terminal's NDK.
              irohSdk = androidPkgs.androidenv.composeAndroidPackages (
                sdkArgs // { ndkVersions = [ "28.2.13676358" ]; }
              );
              ndkRoot = "${irohSdk.androidsdk}/libexec/android-sdk/ndk/28.2.13676358";
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
                    zig
                    selectedSdk.androidsdk
                  ];
                  buildInputs = [ pkgs.sqlite ];
                  JAVA_HOME = "${androidPkgs.jdk21}";
                  ANDROID_HOME = root;
                  ANDROID_SDK_ROOT = root;
                  ANDROID_NDK_ROOT = "${root}/ndk/28.2.13676358";
                  GOATR_IROH_ANDROID = "${irohAndroid}";
                  GOATR_GHOSTTY_SOURCE = "${ghostty}";
                  GOATR_ZIG_PACKAGES = "${zigPackages}";
                  ZIG = "${zig}/bin/zig";
                  JAVA_TOOL_OPTIONS = "-XX:ActiveProcessorCount=2";
                  GOATR_IROH_FFI_SOURCE = "${patchedSource}";
                  GOATR_ADVISORY_DB = "${advisoryDb}";
                  GOATR_READ_GIT_MANIFEST = "${goatrManifest}/manifest.json";
                  GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${root}/build-tools/36.0.0/aapt2";
                };
              mkApk =
                variant: apkName:
                pkgs.writeShellApplication {
                  name = "goatr-apk-${variant}";
                  runtimeInputs = [
                    gradle
                    androidPkgs.jdk21
                  ];
                  text = ''
                    if [[ ! -f settings.gradle.kts || ! -f terminal/native/build.zig ]]; then
                      echo "Run this command from the Goatr checkout; output belongs to that checkout." >&2
                      exit 1
                    fi
                    export JAVA_HOME="${androidPkgs.jdk21}"
                    export JAVA_TOOL_OPTIONS="-XX:ActiveProcessorCount=2"
                    export ANDROID_HOME="${sdkRoot}"
                    export ANDROID_SDK_ROOT="${sdkRoot}"
                    export GOATR_IROH_ANDROID="${irohAndroid}"
                    export GOATR_GHOSTTY_SOURCE="${ghostty}"
                    export GOATR_ZIG_PACKAGES="${zigPackages}"
                    export ZIG="${zig}/bin/zig"
                    export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdkRoot}/build-tools/36.0.0/aapt2"
                    ${lib.optionalString (variant == "Release") "unset GOATR_KEYSTORE GOATR_PASSWORD"}
                    gradle --max-workers=2 --no-parallel ":app:assemble${variant}" "$@"
                    printf '%s\n' "$PWD/app/build/outputs/apk/${lib.toLower variant}/${apkName}"
                  '';
                };
              apkDebug = mkApk "Debug" "app-debug.apk";
              apkReleaseUnsigned = mkApk "Release" "app-release-unsigned.apk";
              mkInstall =
                release:
                let
                  variant = if release then "release" else "debug";
                  builder = if release then apkReleaseUnsigned else apkDebug;
                  builderName = if release then "Release" else "Debug";
                in
                pkgs.writeShellApplication {
                  name = "goatr-install-${variant}";
                  runtimeInputs = [ androidPkgs.jdk21 ];
                  text = ''
                    usage() {
                      echo "Usage: nix run .#install-${variant} -- ${lib.optionalString release "--keystore PATH [--key-alias ALIAS] "}[--serial SERIAL]"
                      echo "Build from this checkout and install on one authorized physical Android device."
                      ${lib.optionalString release ''echo "Use your existing release keystore; apksigner prompts for passwords interactively."''}
                    }
                    fail() { echo "install-${variant}: $*" >&2; exit 1; }
                    if [[ $# == 1 && $1 == --help ]]; then
                      usage
                      exit 0
                    fi
                    serial=""
                    ${lib.optionalString release ''
                      keystore=""
                      key_alias=""
                    ''}
                    while [[ $# -gt 0 ]]; do
                      case "$1" in
                        --serial)
                          [[ -z "$serial" ]] || fail "duplicate --serial"
                          [[ $# -ge 2 && -n "$2" && "$2" != --* ]] || fail "--serial requires a nonempty value"
                          serial="$2"
                          shift 2
                          ;;
                        ${lib.optionalString release ''
                          --keystore)
                            [[ -z "$keystore" ]] || fail "duplicate --keystore"
                            [[ $# -ge 2 && -n "$2" && "$2" != --* ]] || fail "--keystore requires a nonempty path"
                            keystore="$2"
                            shift 2
                            ;;
                          --key-alias)
                            [[ -z "$key_alias" ]] || fail "duplicate --key-alias"
                            [[ $# -ge 2 && -n "$2" && "$2" != --* ]] || fail "--key-alias requires a nonempty value"
                            key_alias="$2"
                            shift 2
                            ;;
                        ''}
                        *) fail "unknown argument: $1 (use --help)" ;;
                      esac
                    done
                    ${lib.optionalString release ''
                      [[ -n "$keystore" ]] || fail "--keystore PATH is required; no release key is generated"
                      [[ -f "$keystore" && -r "$keystore" ]] || fail "keystore must be an existing readable file"
                      [[ -t 0 && -t 1 ]] || fail "release signing requires interactive stdin and stdout terminals for private password prompts"
                    ''}
                    if [[ ! -f settings.gradle.kts || ! -f terminal/native/build.zig ]]; then
                      fail "run from the Goatr checkout; output belongs to that checkout"
                    fi
                    export JAVA_HOME="${androidPkgs.jdk21}"
                    adb="${sdkRoot}/platform-tools/adb"
                    apksigner="${sdkRoot}/build-tools/36.0.0/apksigner"
                    if [[ -z "$serial" ]]; then
                      serial=$("$adb" -d get-serialno) || fail "connect exactly one USB device and authorize this computer, or use --serial"
                      serial="''${serial//$'\r'/}"
                    fi
                    [[ -n "$serial" && "$serial" != unknown && "$serial" != emulator-* ]] || fail "a physical Android device serial is required"
                    check_device() {
                      local state qemu
                      state=$("$adb" -s "$serial" get-state) || fail "device $serial is unavailable; check connection and USB debugging authorization"
                      state="''${state//$'\r'/}"
                      [[ "$state" == device ]] || fail "device $serial is not authorized and online"
                      qemu=$("$adb" -s "$serial" shell getprop ro.kernel.qemu) || fail "cannot check whether $serial is a physical device"
                      qemu="''${qemu//$'\r'/}"
                      [[ -z "$qemu" || "$qemu" == 0 ]] || fail "emulator targets are not supported"
                    }
                    check_device
                    "${builder}/bin/goatr-apk-${builderName}"
                    apk="$PWD/app/build/outputs/apk/${variant}/app-${variant}.apk"
                    ${lib.optionalString release ''
                      unsigned_apk="$PWD/app/build/outputs/apk/release/app-release-unsigned.apk"
                      "${sdkRoot}/build-tools/36.0.0/zipalign" -c -P 16 4 "$unsigned_apk"
                      signing_args=(--ks "$keystore")
                      if [[ -n "$key_alias" ]]; then
                        signing_args+=(--ks-key-alias "$key_alias")
                      fi
                      "$apksigner" sign "''${signing_args[@]}" --out "$apk" "$unsigned_apk"
                    ''}
                    "$apksigner" verify "$apk"
                    check_device
                    "$adb" -s "$serial" install -r "$apk"
                    printf 'Installed %s on %s\n' "$apk" "$serial"
                  '';
                };
            in
            {
              inherit irohAndroid;
              inherit apkDebug apkReleaseUnsigned;
              installDebug = mkInstall false;
              installRelease = mkInstall true;
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
            hc
            patchedSource
            android
            ;
          coreShell = pkgs.mkShell {
            packages = tools ++ [
              rust
              pkgs.zsh
              hc
            ];
            buildInputs = [ pkgs.sqlite ];
            GOATR_HC = "${hc}/bin/hc";
            GOATR_ADVISORY_DB = "${advisoryDb}";
            GOATR_READ_GIT_MANIFEST = "${goatrManifest}/manifest.json";
          };
        }
      );
    in
    assert companionProvenance == ffiProvenance;
    assert builtins.all (
      p: if p.name == "rustls" then lib.versionAtLeast p.version "0.23.45" else p.version == "1.3.0"
    ) companionProvenance;
    {
      nixosModules.default = import ./nix/module.nix { inherit self; };
      packages = lib.mapAttrs (
        system: env:
        {
          inherit (env) goatr hc;
          default = env.goatr;
          iroh-ffi-source = env.patchedSource;
        }
        // lib.optionalAttrs (system == "x86_64-linux") { iroh-android = env.android.irohAndroid; }
      ) environments;
      apps = lib.mapAttrs (
        system: env:
        {
          goatr = {
            type = "app";
            program = "${env.goatr}/bin/goatr";
          };
          hc = {
            type = "app";
            program = "${env.hc}/bin/hc";
          };
          desktop = {
            type = "app";
            program = "${env.pkgs.writeShellScriptBin "goatr-desktop" ''
              exec ${env.goatr}/bin/goatr serve --hc "$@"
            ''}/bin/goatr-desktop";
          };
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
          apk-debug = {
            type = "app";
            program = "${env.android.apkDebug}/bin/goatr-apk-Debug";
          };
          apk-release-unsigned = {
            type = "app";
            program = "${env.android.apkReleaseUnsigned}/bin/goatr-apk-Release";
          };
          install-debug = {
            type = "app";
            program = "${env.android.installDebug}/bin/goatr-install-debug";
          };
          install-release = {
            type = "app";
            program = "${env.android.installRelease}/bin/goatr-install-release";
          };
        }
      ) environments;
      devShells = lib.mapAttrs (
        system: env:
        { default = env.coreShell; } // lib.optionalAttrs (system == "x86_64-linux") env.android.shells
      ) environments;
    };
}
