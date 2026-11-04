package dev.captureone.capture

import dev.captureone.capture.util.FileNaming
import dev.captureone.capture.util.Format
import org.junit.Assert.assertEquals
import org.junit.Test
import java.time.LocalDateTime

/** Same expectations as packages/settings/test/presets.test.ts. */
class NamingAndFormatTest {
    @Test
    fun expandsTemplate() {
        val d = LocalDateTime.of(2026, 11, 4, 9, 5, 7)
        assertEquals("CaptureOne 2026-11-04 09-05-07 Display 1 3", FileNaming.expand("CaptureOne {date} {time} {source} {n}", d, "Display 1", 3))
        assertEquals("CaptureOne 2026-11-04 09-05-07", FileNaming.expand("CaptureOne {date} {time}", d))
        assertEquals("Screen 1", FileNaming.expand("{source} {n}", d))
    }

    @Test
    fun sanitizes() {
        assertEquals("a_b_c__d_e_f_g_h_", FileNaming.sanitize("a<b>c:\"d/e\\f|g?h*"))
        assertEquals("_CON", FileNaming.sanitize("CON"))
        assertEquals("CaptureOne", FileNaming.sanitize("  ...  "))
        assertEquals(120, FileNaming.sanitize("x".repeat(300)).length)
        // Control characters become "_" before whitespace is collapsed (same order as the TS implementation).
        assertEquals("a __ b", FileNaming.sanitize("a \t\n b"))
        assertEquals("a b", FileNaming.sanitize("a  \u00a0 b"))
        assertEquals("tab_x", FileNaming.sanitize("tab\u0001x"))
    }

    @Test
    fun uniqueNames() {
        val existing = setOf("a.mp4", "a (2).mp4")
        assertEquals("a (3).mp4", FileNaming.unique("a", "mp4") { it in existing })
        assertEquals("b.png", FileNaming.unique("b", "png") { false })
    }

    @Test
    fun formatsBytesAndDurations() {
        assertEquals("0 B", Format.bytes(0L))
        assertEquals("1.5 KB", Format.bytes(1536L))
        assertEquals("250 MB", Format.bytes(250L * 1024 * 1024))
        assertEquals("1023 B", Format.bytes(1023L))
        assertEquals("—", Format.bytes(-1L))
        assertEquals("1:02:03", Format.duration(3_723_000L))
        assertEquals("01:05", Format.duration(65_400L))
        assertEquals("00:00", Format.duration(-5L))
    }
}

/** Regression: Android's ICU regex rejects inline flags such as (?U); keep patterns portable. */
class PortableRegexTest {
    @Test
    fun namingPatternsUseNoInlineFlags() {
        val src = RepoFiles.text("native/android-capture/src/main/kotlin/dev/captureone/capture/util/FileNaming.kt")
        org.junit.Assert.assertFalse("(?U) is unsupported on Android", src.contains("(?U)"))
        assertEquals("a b", FileNaming.sanitize("a　 b"))
        assertEquals("x y", FileNaming.sanitize("x﻿y"))
    }
}
