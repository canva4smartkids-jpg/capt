// :capture — Android library with the CaptureOne capture engine (default / AGP mode).
// Depends only on the Android framework plus Maven Central libraries (kotlinx, OkHttp), so the same
// sources also compile in host mode (host.gradle.kts) against android.jar.
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.serialization)
}

val repoRoot = rootProject.extra["captureone.repoRoot"] as String

/**
 * Copies the single source of truth packages/settings/presets.json into generated assets at build
 * time (never hand-copied). The engine reads it as assets/captureone/presets.json.
 */
abstract class CopyEncodingPresets : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val presetsFile: RegularFileProperty

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun copy() {
        val target = outputDir.get().dir("captureone").asFile
        target.deleteRecursively()
        target.mkdirs()
        presetsFile.get().asFile.copyTo(target.resolve("presets.json"), overwrite = true)
    }
}

val copyEncodingPresets = tasks.register<CopyEncodingPresets>("copyEncodingPresets") {
    presetsFile.set(File(repoRoot, "packages/settings/presets.json"))
    outputDir.set(layout.buildDirectory.dir("generated/captureone-assets"))
}

android {
    namespace = "dev.captureone.capture"
    compileSdk = 35
    buildToolsVersion = "35.0.1"

    defaultConfig {
        minSdk = 26
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        consumerProguardFiles("consumer-rules.pro")
    }

    buildTypes {
        release { isMinifyEnabled = false }
    }

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
        warningsAsErrors = false
        checkDependencies = false
        // Strings are intentionally hard-coded in the engine (notification text); apps localise their UI.
        disable += setOf("HardcodedText", "SetTextI18n")
    }
}

androidComponents {
    onVariants { variant ->
        variant.sources.assets?.addGeneratedSourceDirectory(copyEncodingPresets, CopyEncodingPresets::outputDir)
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
        optIn.add("kotlinx.serialization.ExperimentalSerializationApi")
    }
}

dependencies {
    implementation(libs.kotlinx.coroutines.android)
    api(libs.kotlinx.serialization.json)
    api(libs.okhttp)

    testImplementation(libs.junit)
    testImplementation(libs.okhttp.mockwebserver)
    testImplementation(libs.json.schema.validator)
    testImplementation(libs.kotlinx.coroutines.test)

    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.ext.junit)
}
