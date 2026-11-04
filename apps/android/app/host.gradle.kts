// :app in host mode (-Pcaptureone.hostOnly=true; no AGP / Google Maven):
//  * compiles the platform-neutral presentation logic (dev/captureone/android/logic) and runs its tests;
//  * type-checks the platform-neutral Compose screens (dev/captureone/android/ui) against JetBrains
//    Compose for Desktop, which publishes the same androidx.compose.* API on Maven Central.
// The Android glue (activities, services, media3) is only compiled by the AGP build (CI).
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.kotlin.compose)
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
    sourceSets["main"].kotlin.apply {
        setSrcDirs(listOf("src/main/kotlin"))
        include("dev/captureone/android/logic/**", "dev/captureone/android/ui/**")
    }
}

// Compose for Desktop pulls a few androidx JVM artifacts (annotation, collection, lifecycle) that are
// only on Google Maven. They are not needed to type-check this code, so they are left out here.
configurations.matching { it.name.endsWith("ompileClasspath") || it.name.endsWith("untimeClasspath") }.configureEach {
    exclude(group = "androidx.annotation")
    exclude(group = "androidx.collection")
    exclude(group = "androidx.lifecycle")
    exclude(group = "androidx.arch.core")
    exclude(group = "androidx.savedstate")
}

dependencies {
    implementation(project(":capture"))
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.cmp.runtime.desktop)
    implementation(libs.cmp.foundation.desktop)
    implementation(libs.cmp.ui.desktop)
    implementation(libs.cmp.material3.desktop)
    implementation(libs.cmp.material.icons.core.desktop)
    testImplementation(libs.junit)
}

tasks.withType<Test>().configureEach {
    systemProperty("captureone.repoRoot", rootProject.extra["captureone.repoRoot"] as String)
}
tasks.register("testDebugUnitTest") { dependsOn("test") }
