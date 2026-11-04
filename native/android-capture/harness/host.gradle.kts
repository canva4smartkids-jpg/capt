// Engine harness (host mode only): a framework-only Activity (no AndroidX) that drives the real
// :capture engine. Compiled here against android.jar; build-apk.sh turns it into a signed APK with
// the SDK's aapt2 / d8 / zipalign / apksigner. Not a product: the real apps are :app and :tv.
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.jvm)
}

val androidJar = files(rootProject.extra["captureone.androidJar"] as String)

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}

tasks.named<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>("compileKotlin") {
    compilerOptions.noJdk.set(true)
    libraries.from(androidJar)
}

dependencies {
    compileOnly(androidJar)
    implementation(project(":capture"))
    implementation(libs.kotlinx.coroutines.android)
}

tasks.register<Sync>("harnessRuntimeClasspath") {
    from(configurations.runtimeClasspath)
    into(layout.buildDirectory.dir("runtime-classpath"))
}
