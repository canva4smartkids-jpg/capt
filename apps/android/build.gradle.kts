// Root build (default / AGP mode). Plugins are declared once here so AGP and KGP share one classloader.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.serialization) apply false
    alias(libs.plugins.kotlin.compose) apply false
}

/** Monorepo root (…/capt). Unit tests read shared contract fixtures from here. */
val repoRoot: File = rootDir.resolve("../..").canonicalFile
extra["captureone.repoRoot"] = repoRoot.absolutePath
