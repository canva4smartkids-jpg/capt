// :capture in host mode (no AGP): compiles the same sources with Kotlin/JVM against the SDK's
// android.jar (with -no-jdk so java.* also resolves from android.jar, as on a device) and runs
// the JVM unit tests. Used only where Google Maven is unreachable; CI uses build.gradle.kts.
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    `java-library`
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.kotlin.serialization)
}

val androidJar = files(rootProject.extra["captureone.androidJar"] as String)
val repoRoot = rootProject.extra["captureone.repoRoot"] as String

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
        optIn.add("kotlinx.serialization.ExperimentalSerializationApi")
    }
}

tasks.named<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>("compileKotlin") {
    // Resolve java.* from android.jar only: catches use of JDK APIs that do not exist on Android.
    compilerOptions.noJdk.set(true)
    libraries.from(androidJar)
}

dependencies {
    compileOnly(androidJar)
    implementation(libs.kotlinx.coroutines.android)
    api(libs.kotlinx.serialization.json)
    api(libs.okhttp)

    testCompileOnly(androidJar)
    testImplementation(libs.junit)
    testImplementation(libs.okhttp.mockwebserver)
    testImplementation(libs.json.schema.validator)
    testImplementation(libs.kotlinx.coroutines.test)
}

tasks.withType<Test>().configureEach {
    systemProperty("captureone.repoRoot", repoRoot)
    testLogging { events("failed"); exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL }
}

// Same task names as the AGP build so the documented commands work in both modes.
tasks.register("testDebugUnitTest") { dependsOn("test") }

/** Jar of the compiled engine classes, used by tools/host-verify.sh to dex (d8 --min-api 26) and lint. */
tasks.register<Sync>("hostRuntimeClasspath") {
    from(configurations.runtimeClasspath)
    into(layout.buildDirectory.dir("host-runtime-classpath"))
}
