// SPDX-License-Identifier: GPL-3.0-or-later
plugins {
    `java-library`
    id("org.jetbrains.kotlin.jvm")
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        optIn.add("kotlin.ExperimentalUnsignedTypes")
    }
    sourceSets.named("main") {
        kotlin.srcDir(rootProject.layout.buildDirectory.dir("iroh-native/kotlin"))
    }
}

sourceSets.named("main") {
    resources.srcDir(rootProject.layout.buildDirectory.dir("iroh-native/share"))
}

tasks.named("processResources") {
    dependsOn(rootProject.tasks.named("stageIroh"))
}

dependencies {
    compileOnly("net.java.dev.jna:jna:5.19.1")
    testRuntimeOnly("net.java.dev.jna:jna:5.19.1")
    api("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.11.0")
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    dependsOn(rootProject.tasks.named("stageIroh"))
}
