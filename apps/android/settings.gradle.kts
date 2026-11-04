// CaptureOne Android Gradle root.
//
//   :app      apps/android/app            phone / tablet app   (dev.captureone.android)
//   :tv       apps/android-tv             Android TV / Google TV app (dev.captureone.tv)
//   :capture  native/android-capture      Android library with the capture engine
//
// Two modes:
//  * default        – the real Android build (AGP, Compose, media3). Needs Google Maven.
//  * host mode      – `-Pcaptureone.hostOnly=true`. Used where Google Maven is unreachable (this
//                     build container). Compiles the :capture engine with the plain Kotlin JVM plugin
//                     against the SDK's android.jar (no AGP), type-checks the platform-neutral Compose
//                     UI code against JetBrains Compose for Desktop, and runs every JVM unit test.
//                     It produces no APKs. See README.md.
pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
    }
}

rootProject.name = "captureone-android"

val hostOnly = providers.gradleProperty("captureone.hostOnly").map { it.toBoolean() }.getOrElse(false)

include(":capture")
project(":capture").projectDir = file("../../native/android-capture")
include(":app")
include(":tv")
project(":tv").projectDir = file("../android-tv")

if (hostOnly) {
    rootProject.buildFileName = "host.gradle.kts"
    project(":capture").buildFileName = "host.gradle.kts"
    project(":app").buildFileName = "host.gradle.kts"
    project(":tv").buildFileName = "host.gradle.kts"
}
