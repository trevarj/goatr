// SPDX-License-Identifier: GPL-3.0-or-later
import com.android.build.api.artifact.SingleArtifact
import org.gradle.api.DefaultTask
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.serialization")
    id("org.jetbrains.kotlin.plugin.compose")
}

/** AGP's test-manifest producer otherwise replaces the standalone declaration with its default runner. */
abstract class OwnedInstrumentationManifest : DefaultTask() {
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE)
    abstract val inputManifest: RegularFileProperty
    @get:OutputFile
    abstract val outputManifest: RegularFileProperty
    @TaskAction
    fun produce() {
        val factory=javax.xml.parsers.DocumentBuilderFactory.newInstance().apply {
            isNamespaceAware=true
            setFeature("http://apache.org/xml/features/disallow-doctype-decl",true)
            setFeature(javax.xml.XMLConstants.FEATURE_SECURE_PROCESSING,true)
            setAttribute(javax.xml.XMLConstants.ACCESS_EXTERNAL_DTD,"")
            setAttribute(javax.xml.XMLConstants.ACCESS_EXTERNAL_SCHEMA,"")
        }
        val document=factory.newDocumentBuilder().parse(inputManifest.get().asFile)
        val android="http://schemas.android.com/apk/res/android"
        val runners=linkedMapOf(
            "io.github.trevarj.goatr.OwnedInstrumentationRunner" to "io.github.trevarj.goatr.debug",
            "io.github.trevarj.goatr.OwnedUiAutomationRunner" to "io.github.trevarj.goatr.debug.test",
        )
        val existing=document.getElementsByTagName("instrumentation")
        while(existing.length>0) {
            val node=existing.item(0) as org.w3c.dom.Element
            check(node.getAttributeNS(android,"name") in runners) {"Unexpected instrumentation producer"}
            node.parentNode.removeChild(node)
        }
        for((name,target) in runners)document.documentElement.appendChild(document.createElement("instrumentation").apply {
            setAttributeNS(android,"android:name",name)
            setAttributeNS(android,"android:targetPackage",target)
            setAttributeNS(android,"android:label",if(target.endsWith(".test")) "Goatr owned UI automation" else "Goatr owned product observer")
        })
        val file=outputManifest.get().asFile
        file.parentFile.mkdirs()
        javax.xml.transform.TransformerFactory.newInstance().apply {
            setFeature(javax.xml.XMLConstants.FEATURE_SECURE_PROCESSING,true)
            setAttribute(javax.xml.XMLConstants.ACCESS_EXTERNAL_DTD,"")
            setAttribute(javax.xml.XMLConstants.ACCESS_EXTERNAL_STYLESHEET,"")
        }.newTransformer().transform(javax.xml.transform.dom.DOMSource(document),javax.xml.transform.stream.StreamResult(file))
    }
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

androidComponents {
    onVariants(selector().withBuildType("debug")) {variant->
        variant.androidTest?.let {test->
            val producer=tasks.register<OwnedInstrumentationManifest>("produce${variant.name.replaceFirstChar {it.uppercase()}}OwnedInstrumentations")
            test.artifacts.use(producer).wiredWithFiles(
                OwnedInstrumentationManifest::inputManifest,OwnedInstrumentationManifest::outputManifest,
            ).toTransform(SingleArtifact.MERGED_MANIFEST)
        }
    }
}
kotlin { jvmToolchain(21) }

if (providers.gradleProperty("goatrPairedFeedbackOnly").orNull == "true") {
    // Keep the paired feedback APK independent of unrelated instrumentation sources.
    (android.sourceSets.getByName("androidTest").kotlin as org.gradle.api.tasks.util.PatternFilterable).setIncludes(
        listOf(
            "**/OwnedInstrumentationRunner.kt",
            "**/PairedDeviceFeedback.kt",
            "**/PhysicalMvpObserver.kt",
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
