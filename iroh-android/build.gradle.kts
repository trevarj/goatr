// SPDX-License-Identifier: GPL-3.0-or-later
plugins {
    id("com.android.library")
}

// AGP 9 supplies built-in Kotlin; the root JVM plugin pins its shared KGP to 2.4.20.
android {
    namespace = "computer.iroh.android"
    compileSdk = 37
    buildToolsVersion = "36.0.0"
    ndkVersion = "28.2.13676358"
    defaultConfig {
        minSdk = 26
        ndk.abiFilters += listOf("arm64-v8a", "x86_64")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
    packaging {
        jniLibs.useLegacyPackaging = false
    }
}

kotlin {
    jvmToolchain(21)
}

androidComponents.onVariants { variant ->
    variant.sources.kotlin?.addStaticSourceDirectory(
        rootProject.layout.buildDirectory.dir("iroh-native/android").get().asFile.path
    )
    variant.sources.jniLibs?.addStaticSourceDirectory(
        rootProject.layout.buildDirectory.dir("iroh-native/jniLibs").get().asFile.path
    )
}

dependencies {
    api(project(":iroh-jvm")) {
        exclude(group = "net.java.dev.jna", module = "jna")
    }
    api("net.java.dev.jna:jna:5.19.1@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
}

tasks.named("preBuild") {
    dependsOn(rootProject.tasks.named("stageIroh"))
}
