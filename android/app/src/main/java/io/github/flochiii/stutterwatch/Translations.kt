package io.github.flochiii.stutterwatch

import android.content.Context
import org.json.JSONObject

class Translations(private val context: Context, language: String) {
    private fun load(path: String) = runCatching { context.assets.open(path).bufferedReader().use { JSONObject(it.readText()) } }.getOrDefault(JSONObject())
    private val fallback=load("mobile/en.json")
    private val mobile=load("mobile/$language.json")
    private val catalog=load("locales/$language.json")
    operator fun get(key: String): String = when {
        mobile.has(key) -> mobile.getString(key)
        catalog.has(key) -> catalog.getString(key)
        fallback.has(key) -> fallback.getString(key)
        else -> key
    }
    companion object {
        val languages=listOf("de" to "Deutsch","en" to "English","es" to "Español","fr" to "Français","pl" to "Polski","pt" to "Português","ru" to "Русский","uk" to "Українська","zh" to "中文","ja" to "日本語")
    }
}
