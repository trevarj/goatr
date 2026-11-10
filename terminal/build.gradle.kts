// The terminal view: libghostty-vt drawn by an Android View. Gradle stages pinned
// Ghostty and this Zig JNI bridge under build/native-source; Nix supplies the
// offline source/dependency graph and Zig toolchain. The
// view started from expo-libghostty 0.8.1 (MIT), without its Expo wrapper.

plugins {
    alias(libs.plugins.android.library)
}

/** Each ABI's Zig target, at minSdk's API level. */
val ZIG_TARGETS = mapOf("arm64-v8a" to "aarch64-linux-android.29", "x86_64" to "x86_64-linux-android.29")

/** Where each ABI's `libherdrterminal.so` lands, for packaging. */
val jniOut = layout.buildDirectory.dir("jniLibs")

android {
    namespace = "dev.herdr.terminal"
    compileSdk = 37
    // For its Bionic headers and libraries; there's no C/C++ compiled with it.
    ndkVersion = "27.1.12297006"
    defaultConfig {
        minSdk = 29
        // The only ABIs built; 32-bit isn't supported.
        ndk {
            abiFilters += ZIG_TARGETS.keys
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    // Drawing tests run on the JVM: Robolectric with its native graphics,
    // the bundled fonts as assets.
    testOptions.unitTests.isIncludeAndroidResources = true
}

androidComponents.onVariants { variant ->
    variant.sources.jniLibs?.addStaticSourceDirectory(jniOut.get().asFile.path)
    // Fonts for glyphs absent from the system fallback.
    variant.sources.assets?.addStaticSourceDirectory("vendor/fonts")
}

dependencies {
    implementation(libs.core.ktx)
    testImplementation(kotlin("test-junit"))
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
}

// Sources and offline Zig packages are pinned by the flake, never a missing submodule.
val ghosttySource = providers.environmentVariable("GOATR_GHOSTTY_SOURCE").orElse(providers.provider {
    throw GradleException("Use nix run .#apk-debug or the Android dev shell: missing pinned Ghostty source")
})
val zigPackages = providers.environmentVariable("GOATR_ZIG_PACKAGES").orElse(providers.provider {
    throw GradleException("Use the Android Nix environment: missing offline Zig packages")
})
val prepared = layout.buildDirectory.dir("native-source")
val prepareNative = tasks.register<Sync>("prepareNative") {
    from("native") { into("native") }
    from(ghosttySource) { into("ghostty") }
    into(prepared)
    // Nix source directories are read-only; Sync must populate a writable staging tree.
    dirPermissions { unix("0755") }
    eachFile {
        val originalPermissions = permissions.toUnixNumeric()
        permissions { unix(originalPermissions or 0b010_000_000) }
    }
}
val zigBuilds = ZIG_TARGETS.map { (abi, target) ->
    tasks.register<Exec>("zigJni-$abi") {
        dependsOn(prepareNative)
        val out = jniOut.map { it.dir(abi) }
        val cache = layout.buildDirectory.dir("zig-cache")
        val ndk = androidComponents.sdkComponents.ndkDirectory
        workingDir = prepared.get().dir("native").asFile
        inputs.dir(prepared)
        inputs.dir(zigPackages)
        outputs.dir(out)
        doFirst {
            environment("ANDROID_NDK_HOME", ndk.get().asFile.absolutePath)
            commandLine(
                System.getenv("ZIG") ?: "zig", "build",
                "-j2", "--system", zigPackages.get(), "-Dtarget=$target", "-Doptimize=ReleaseFast",
                "--prefix", layout.buildDirectory.dir("zig-out/$abi").get().asFile.absolutePath,
                "--prefix-lib-dir", out.get().asFile.absolutePath,
                "--cache-dir", cache.get().asFile.absolutePath,
            )
        }
    }
}
tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }.configureEach { dependsOn(zigBuilds) }
