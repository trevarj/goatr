// Shared Herdr protocol client. The Android product and JVM checks use its JVM target.

import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.kotlin.serialization)
}

kotlin {
    // Java 17 bytecode is usable by every Android module.
    jvm {
        compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
    }

    sourceSets {
        commonMain.dependencies {
            api(libs.kotlinx.serialization.json)
            api(libs.kotlinx.coroutines.core)
        }
        commonTest.dependencies {
            implementation(kotlin("test"))
            implementation(libs.kotlinx.io.core)
            implementation(libs.kotlinx.coroutines.test)
        }
    }
}

val fixtures = layout.projectDirectory.dir("fixtures").asFile.absolutePath
tasks.withType<Test>().configureEach { environment("HERDR_FIXTURES", fixtures) }
