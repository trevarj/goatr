// SPDX-License-Identifier: GPL-3.0-or-later
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.serialization")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "io.github.trevarj.goatr"
    compileSdk = 37
    buildToolsVersion = "36.0.0"
    ndkVersion = "28.2.13676358"
    defaultConfig {
        applicationId = "io.github.trevarj.goatr"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "io.github.trevarj.goatr.OwnedInstrumentationRunner"
        ndk.abiFilters += listOf("arm64-v8a", "x86_64")
    }
    buildTypes { debug { applicationIdSuffix = ".debug" } }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
    packaging { jniLibs.useLegacyPackaging = false }
    buildFeatures { compose = true }
    sourceSets {
        getByName("test").kotlin.directories.add("src/sharedTest/kotlin")
        getByName("androidTest").kotlin.directories.add("src/sharedTest/kotlin")
        getByName("androidTest").assets.directories.add(rootProject.file("protocol/fixtures").absolutePath)
    }
    testOptions { unitTests.all { it.systemProperty("goatr.fixtures", rootProject.file("protocol/fixtures").absolutePath) } }
}
kotlin { jvmToolchain(21) }

if (providers.gradleProperty("goatrPairedFeedbackOnly").orNull == "true") {
    // Keep the paired feedback APK independent of unrelated instrumentation sources.
    (android.sourceSets.getByName("androidTest").kotlin as org.gradle.api.tasks.util.PatternFilterable).setIncludes(
        listOf(
            "**/OwnedInstrumentationRunner.kt",
            "**/PairedDeviceFeedback.kt",
        ),
    )
}

dependencies {
    implementation(project(":iroh-android"))
    implementation("org.unifiedpush.android:connector:3.3.5")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")
    implementation("androidx.camera:camera-core:1.6.1")
    implementation("androidx.camera:camera-camera2:1.6.1")
    implementation("androidx.camera:camera-lifecycle:1.6.1")
    implementation("androidx.camera:camera-view:1.6.1")
    implementation("com.google.zxing:core:3.5.4")
    implementation("io.noties.markwon:core:4.6.2")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation(platform("androidx.compose:compose-bom:2026.09.00"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
