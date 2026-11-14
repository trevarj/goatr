plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "io.github.trevarj.goatr.app"
    compileSdk = 37
    defaultConfig {
        applicationId = "io.github.trevarj.goatr"
        minSdk = 29
        targetSdk = 37
        versionCode = 4
        versionName = "0.2.2"
        // The terminal's libghostty-vt is only built for these.
        ndk {
            abiFilters += setOf("arm64-v8a", "x86_64")
        }
    }
    // Debug installs beside the release with its own id and link scheme.
    buildTypes {
        debug {
            applicationIdSuffix = ".dev"
            versionNameSuffix = "-dev"
            resValue("string", "app_name", "Goatr dev")
            manifestPlaceholders["scheme"] = "goatr-dev"
            buildConfigField("String", "SCHEME", "\"goatr-dev\"")
        }
        release {
            resValue("string", "app_name", "Goatr")
            manifestPlaceholders["scheme"] = "goatr"
            buildConfigField("String", "SCHEME", "\"goatr\"")
            // Deliberately unsigned. Signing belongs to the distributor, never this checkout.
        }
    }
    buildFeatures {
        compose = true
        resValues = true
        buildConfig = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
    packaging {
        resources.excludes += setOf("META-INF/versions/9/OSGI-INF/MANIFEST.MF", "META-INF/*.md")
    }
}

dependencies {
    implementation(project(":core"))
    implementation(project(":terminal"))
    implementation(project(":ssh"))
    implementation(project(":iroh-android"))
    implementation(libs.kotlinx.serialization.json)
    implementation("androidx.camera:camera-core:1.6.1")
    implementation("androidx.camera:camera-camera2:1.6.1")
    implementation("androidx.camera:camera-lifecycle:1.6.1")
    implementation("androidx.camera:camera-view:1.6.1")
    implementation("com.google.zxing:core:3.5.4")
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    testImplementation(kotlin("test-junit"))
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.navigation.compose)
    implementation(libs.datastore.preferences)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.core.ktx)
    implementation(libs.biometric)
    implementation(libs.fragment)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidsvg)
    implementation(libs.camera.core)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.zxing.core)
}
