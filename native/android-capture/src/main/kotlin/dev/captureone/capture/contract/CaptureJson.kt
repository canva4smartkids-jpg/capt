package dev.captureone.capture.contract

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * The one JSON configuration used for every contract type.
 *  - `encodeDefaults`: zod defaults (e.g. `audioTracks: []`, `warnings: []`) are always present.
 *  - `explicitNulls = false`: optional fields are omitted instead of written as `null`
 *    (the schemas type them as e.g. `string`, so `null` would be invalid).
 *  - `ignoreUnknownKeys`: newer hubs may add fields.
 */
val CaptureJson: Json = Json {
    encodeDefaults = true
    explicitNulls = false
    ignoreUnknownKeys = true
    coerceInputValues = true
}

/** Exception carrying a contract [CaptureError]; thrown across engine/service boundaries. */
class CaptureException(val error: CaptureError, cause: Throwable? = null) : Exception(error.message, cause) {
    constructor(
        code: CaptureErrorCode,
        message: String,
        recoverable: Boolean = false,
        cause: Throwable? = null,
        details: Map<String, String>? = null,
    ) : this(
        CaptureError(code, message, recoverable, details?.let { d -> JsonObject(d.mapValues { JsonPrimitive(it.value) }) }),
        cause,
    )

    val code: CaptureErrorCode get() = error.code
}

/** Normalises anything thrown into a [CaptureError] (mirror of `toCaptureError`). */
fun Throwable.toCaptureError(): CaptureError = when (this) {
    is CaptureException -> error
    is SecurityException -> CaptureError(CaptureErrorCode.PERMISSION_DENIED, message ?: "Permission denied", false)
    is java.io.IOException -> CaptureError(CaptureErrorCode.IO_ERROR, message ?: "I/O error", false)
    else -> CaptureError(CaptureErrorCode.INTERNAL, message ?: (this::class.simpleName ?: "Unknown error"), false)
}
