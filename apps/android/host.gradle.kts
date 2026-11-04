// Root build in host mode (-Pcaptureone.hostOnly=true): no Android Gradle Plugin, only Kotlin/JVM.
plugins {
    alias(libs.plugins.kotlin.jvm) apply false
    alias(libs.plugins.kotlin.serialization) apply false
    alias(libs.plugins.kotlin.compose) apply false
}

val repoRoot: File = rootDir.resolve("../..").canonicalFile
extra["captureone.repoRoot"] = repoRoot.absolutePath

/** Locates android.jar of the compile SDK without AGP (ANDROID_HOME, ANDROID_SDK_ROOT or local.properties). */
fun findAndroidJar(): File {
    val local = rootDir.resolve("local.properties")
    val fromLocal = if (local.isFile) {
        java.util.Properties().apply { local.inputStream().use { load(it) } }.getProperty("sdk.dir")
    } else null
    val sdk = fromLocal ?: System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
        ?: error("Android SDK not found: set ANDROID_HOME or sdk.dir in local.properties")
    val jar = File(sdk, "platforms/android-35/android.jar")
    check(jar.isFile) { "android.jar for API 35 not found at $jar" }
    return jar
}
extra["captureone.androidJar"] = findAndroidJar().absolutePath
