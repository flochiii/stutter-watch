package io.github.flochiii.stutterwatch

import org.junit.Assert.*
import org.junit.Test
import java.io.File

class CatalogTest {
    @Test fun everyMobileLanguageHasTheSameNonemptyLabels() {
        val directory = File("src/main/assets/mobile")
        val entries = Regex("\"([^\"]+)\"\\s*:\\s*\"((?:[^\"\\\\]|\\\\.)*)\"")
        fun catalog(file: File) = entries.findAll(file.readText()).associate { it.groupValues[1] to it.groupValues[2] }
        val english = catalog(File(directory, "en.json"))
        assertTrue(english.size >= 90)
        val expected = setOf("de", "en", "es", "fr", "pl", "pt", "ru", "uk", "zh", "ja")
        assertEquals(expected, directory.listFiles()!!.map { it.nameWithoutExtension }.toSet())
        for (language in expected) {
            val translated = catalog(File(directory, "$language.json"))
            assertEquals(language, english.keys, translated.keys)
            assertTrue(language, translated.values.all { it.isNotBlank() })
            if (language != "en") assertNotEquals(language, english.getValue("no_data"), translated.getValue("no_data"))
        }
    }
}
