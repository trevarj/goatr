// SSH and the phone's key. Its own module so the key stays behind `internal`:
// the app (and its UI) can ask for the public half and open logins with the
// key, but has no way to read the private half.

plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "dev.herdr.ssh"
    compileSdk = 37
    defaultConfig {
        minSdk = 29
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":core"))
    implementation(libs.jsch)
    implementation(libs.bouncycastle)
    implementation(libs.kotlinx.coroutines.android)
}
