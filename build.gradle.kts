// SPDX-License-Identifier: GPL-3.0-or-later
import groovy.json.JsonSlurper
import java.security.MessageDigest

buildscript {
    configurations.getByName("classpath").resolutionStrategy.activateDependencyLocking()
}

plugins {
    id("com.android.library") version "9.4.1" apply false
    id("com.android.application") version "9.4.1" apply false
    id("org.jetbrains.kotlin.jvm") version "2.4.20" apply false
    id("org.jetbrains.kotlin.plugin.serialization") version "2.4.20" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.20" apply false
}

allprojects {
    dependencyLocking {
        lockAllConfigurations()
        lockMode.set(LockMode.STRICT)
    }
    buildscript.configurations.configureEach {
        resolutionStrategy.activateDependencyLocking()
    }
    tasks.register("resolveDependencies") {
        doLast {
            configurations.filter { it.isCanBeResolved }.forEach { configuration ->
                configuration.incoming.artifactView {
                    componentFilter { it is org.gradle.api.artifacts.component.ModuleComponentIdentifier }
                }.files.files
            }
        }
    }
}

fun sha256(file: File): String {
    val digest = MessageDigest.getInstance("SHA-256")
    file.inputStream().use { input ->
        val buffer = ByteArray(8192)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            digest.update(buffer, 0, count)
        }
    }
    return digest.digest().joinToString("") { "%02x".format(it) }
}

val nativeRoot = providers.environmentVariable("GOATR_IROH_ANDROID").map(::file).orElse(
    providers.provider {
        throw GradleException("Missing GOATR_IROH_ANDROID; run nix build path:.#iroh-android and use the Android Nix shell")
    }
)

// Nix is the sole native producer; Gradle only verifies and stages its output.
tasks.register<Sync>("stageIroh") {
    from(nativeRoot)
    inputs.files(
        "third_party/iroh-ffi/source.json",
        "third_party/iroh-ffi/Cargo.lock",
        "third_party/iroh-ffi/patches/0001-secure-iroh-1.3.patch",
        "companion/Cargo.lock",
        "companion/dependencies.json"
    )
    into(layout.buildDirectory.dir("iroh-native"))
    dirPermissions { unix("0755") }
    filePermissions { unix("0644") }
    doFirst {
        val root = nativeRoot.get().canonicalFile
        check(root.path.startsWith("/nix/store/")) { "GOATR_IROH_ANDROID must name an immutable Nix output" }
        val parser = JsonSlurper()
        val expected = parser.parse(file("third_party/iroh-ffi/source.json"))
        val manifestFile = root.resolve("manifest.json")
        check(manifestFile.isFile) { "Missing native manifest; run nix build path:.#iroh-android" }
        val manifest = parser.parse(manifestFile) as Map<*, *>
        check(manifest["schema"] == 1 && manifest["identity"] == expected) {
            "Mismatched native source/patch/lock; run nix build path:.#iroh-android"
        }
        check(manifest["abis"] == listOf("arm64-v8a", "x86_64")) { "Unexpected native ABI set" }
        check(sha256(file("third_party/iroh-ffi/Cargo.lock")) == (expected as Map<*, *>)["lockSha256"])
        check(sha256(file("third_party/iroh-ffi/patches/0001-secure-iroh-1.3.patch")) == expected["patchSha256"])
        val companion = parser.parse(file("companion/dependencies.json")) as Map<*, *>
        check(companion["lockSha256"] == sha256(file("companion/Cargo.lock"))) {
            "Companion dependency metadata is stale; resolve and retain the updated graph"
        }
        val shared = (companion["packages"] as List<*>).map { it as Map<*, *> }
            .filter { it["name"] in listOf("iroh", "iroh-base", "iroh-relay", "rustls") }
            .map { packageInfo -> listOf("name", "version", "source", "checksum").associateWith { packageInfo[it] } }
        check(manifest["sharedProvenance"] == shared) { "Companion and Android transport/security provenance differ" }
        val artifacts = manifest["artifacts"] as Map<*, *>
        check(artifacts.keys.containsAll(listOf(
            "jniLibs/arm64-v8a/libiroh_ffi.so", "jniLibs/x86_64/libiroh_ffi.so"
        ))) { "Incomplete native output" }
        check(artifacts.keys.any { it.toString().startsWith("kotlin/") && it.toString().endsWith(".kt") })
        check(artifacts["android/computer/iroh/IrohAndroid.kt"] == expected["contextSha256"])
        artifacts.forEach { (relative, hash) ->
            val artifact = root.resolve(relative.toString()).canonicalFile
            check(artifact.toPath().startsWith(root.toPath()) && artifact.isFile && sha256(artifact) == hash) {
                "Native artifact hash mismatch: $relative; run nix build path:.#iroh-android"
            }
        }
    }
}
