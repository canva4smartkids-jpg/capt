package dev.captureone.capture

import java.io.File

/** Locates shared contract files in the monorepo (tests read them in place, never copies). */
object RepoFiles {
    val root: File by lazy {
        System.getProperty("captureone.repoRoot")?.let { File(it) }
            ?: generateSequence(File("").absoluteFile) { it.parentFile }.first { File(it, "packages/capture-contracts").isDirectory }
    }

    fun file(relative: String): File = File(root, relative).also { require(it.isFile) { "missing contract file $it" } }

    fun text(relative: String): String = file(relative).readText()

    val presetsJson: String get() = text("packages/settings/presets.json")
}
