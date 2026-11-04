// :app — CaptureOne for Android phones and tablets (Jetpack Compose, Material 3).
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

val repoRoot = rootProject.extra["captureone.repoRoot"] as String

/**
 * Release signing comes from outside the repository only: Gradle properties (e.g. in
 * ~/.gradle/gradle.properties) or environment variables. Without them the release APK is unsigned.
 */
fun signingValue(prop: String, env: String): String? =
    (findProperty(prop) as String?)?.takeIf { it.isNotBlank() } ?: System.getenv(env)?.takeIf { it.isNotBlank() }

val releaseStore = signingValue("captureone.signing.storeFile", "CAPTUREONE_KEYSTORE")

android {
    namespace = "dev.captureone.android"
    compileSdk = 35
    buildToolsVersion = "35.0.1"

    defaultConfig {
        applicationId = "dev.captureone.android"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        if (releaseStore != null) {
            create("release") {
                storeFile = file(releaseStore)
                storePassword = signingValue("captureone.signing.storePassword", "CAPTUREONE_KEYSTORE_PASSWORD")
                keyAlias = signingValue("captureone.signing.keyAlias", "CAPTUREONE_KEY_ALIAS")
                keyPassword = signingValue("captureone.signing.keyPassword", "CAPTUREONE_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            // R8 is off until minified builds are verified on devices (kotlinx.serialization, media3).
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release")
        }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    testOptions {
        unitTests {
            isReturnDefaultValues = true
            all { it.systemProperty("captureone.repoRoot", repoRoot) }
        }
    }

    lint {
        abortOnError = true
        checkDependencies = true
        disable += setOf("HardcodedText", "SetTextI18n")
    }

    packaging {
        resources.excludes += setOf("/META-INF/{AL2.0,LGPL2.1}", "/META-INF/versions/9/OSGI-INF/MANIFEST.MF")
    }
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}

dependencies {
    implementation(project(":capture"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.core)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.media3.exoplayer)
    implementation(libs.media3.ui)
    implementation(libs.kotlinx.coroutines.android)

    testImplementation(libs.junit)
}
