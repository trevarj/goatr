rootProject.name = "goatr"

pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
        google()
    }
}

dependencyResolutionManagement {
    repositories {
        mavenCentral()
        google()
    }
}

include(":core", ":terminal", ":ssh", ":app")

include(":iroh-android", ":iroh-jvm")
