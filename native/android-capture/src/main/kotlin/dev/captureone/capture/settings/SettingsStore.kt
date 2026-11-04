package dev.captureone.capture.settings

import android.content.Context
import android.content.SharedPreferences

/** Persists [CaptureSettings] and small engine facts in SharedPreferences. */
class SettingsStore(context: Context) {
    private val prefs: SharedPreferences = context.applicationContext.getSharedPreferences("captureone", Context.MODE_PRIVATE)

    fun load(): CaptureSettings = CaptureSettings.fromJson(prefs.getString(KEY_SETTINGS, null))

    fun save(settings: CaptureSettings) {
        prefs.edit().putString(KEY_SETTINGS, settings.toJson()).apply()
    }

    /** Encoders that produced a recording which passed MediaExtractor validation on this device. */
    fun verifiedEncoders(): Set<String> = prefs.getStringSet(KEY_VERIFIED, emptySet()).orEmpty()

    fun markEncoderVerified(name: String) {
        val s = verifiedEncoders().toMutableSet()
        if (s.add(name)) prefs.edit().putStringSet(KEY_VERIFIED, s).apply()
    }

    /** Whether a runtime permission was requested before (distinguishes "prompt" from "denied"). */
    fun wasRequested(permission: String): Boolean = prefs.getBoolean("asked:$permission", false)

    fun markRequested(permission: String) {
        prefs.edit().putBoolean("asked:$permission", true).apply()
    }

    /** Last companion hub (address + token) for the TV app. */
    fun companionHub(): Pair<String, String>? {
        val a = prefs.getString(KEY_HUB_ADDRESS, null) ?: return null
        val t = prefs.getString(KEY_HUB_TOKEN, null) ?: return null
        return a to t
    }

    fun saveCompanionHub(address: String, token: String) {
        prefs.edit().putString(KEY_HUB_ADDRESS, address).putString(KEY_HUB_TOKEN, token).apply()
    }

    fun clearCompanionHub() {
        prefs.edit().remove(KEY_HUB_ADDRESS).remove(KEY_HUB_TOKEN).apply()
    }

    private companion object {
        const val KEY_SETTINGS = "settings"
        const val KEY_VERIFIED = "verifiedEncoders"
        const val KEY_HUB_ADDRESS = "hubAddress"
        const val KEY_HUB_TOKEN = "hubToken"
    }
}
