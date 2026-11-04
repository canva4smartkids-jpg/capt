// :tv in host mode (-Pcaptureone.hostOnly=true; no AGP / Google Maven):
//  * compiles and tests the platform-neutral TV logic (dev/captureone/tv/logic);
//  * type-checks the TV screens (dev/captureone/tv/ui) against Compose for Desktop, with
//    src/hostStub standing in for the androidx.tv.material3 adapter (src/main/.../tvkit).
// The real tvkit, activity and players are compiled only by the AGP build (CI).
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
        setSrcDirs(listOf("src/main/kotlin", "src/hostStub/kotlin"))
        include("dev/captureone/tv/logic/**", "dev/captureone/tv/ui/**", "dev/captureone/tv/tvkit/**")
        // the real adapter needs androidx.tv.material3 (Google Maven only)
        exclude { it.file.path.replace('\\', '/').contains("/src/main/kotlin/dev/captureone/tv/tvkit/") }
    }
}

configurations.matching { it.name.endsWith("ompileClasspath") || it.name.endsWith("untimeClasspath") }.configureEach {
    exclude(group = "androidx.annotation")
    exclude(group = "androidx.collection")
    exclude(group = "androidx.lifecycle")
    exclude(group = "androidx.arch.core")
    exclude(group = "androidx.savedstate")
}

dependencies {
    implementation(project(":capture"))
    implementation(libs.cmp.runtime.desktop)
    implementation(libs.cmp.foundation.desktop)
    implementation(libs.cmp.ui.desktop)
    implementation(libs.cmp.material3.desktop)
    testImplementation(libs.junit)
}

tasks.register("testDebugUnitTest") { dependsOn("test") }
