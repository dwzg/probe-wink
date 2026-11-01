package de.wtzg.probewink

import java.util.UUID

/**
 * The module's phone interface, as implemented in firmware/wink-app/src/ble_phone.rs
 * and firmware/wink-core/src/command.rs. Keep the two in sync.
 *
 * - Command characteristic: write (with response), ASCII text, at most 32 bytes.
 * - State characteristic: read + notify, ASCII text: a state word, or
 *   `error: <reason>` (`error: resting <s>`), `remote: <status>`, `version: <x.y.z>`,
 *   `drivelock: on|off`.
 * Both need an encrypted, authenticated (passkey) link from a bonded phone.
 */
object Protocol {
    val SERVICE: UUID = UUID.fromString("7a1e0001-5c0b-4e57-9b1a-70726f626577")
    val COMMAND: UUID = UUID.fromString("7a1e0002-5c0b-4e57-9b1a-70726f626577")
    val STATE: UUID = UUID.fromString("7a1e0003-5c0b-4e57-9b1a-70726f626577")
    val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

    /** Advertised short name and GAP name. */
    val NAMES = setOf("PWink", "ProbeWink")

    const val MAX_LEN = 32
    const val ALT_MAX_S = 10       // config.rs: alt_max_s
    const val ALT_DEFAULT_S = 8    // config.rs: alt_default_s
    const val REST_S = 20          // config.rs: rest after an alternation

    val STATES = listOf("asleep", "idle", "wink", "alternating", "resting", "overvoltage", "undervoltage")

    fun alt(seconds: Int) = "ALT ${seconds.coerceIn(1, ALT_MAX_S)}"

    fun driveLock(on: Boolean) = if (on) "DRIVELOCK ON" else "DRIVELOCK OFF"
}

/** What a button, widget or tile can ask for. `ALTERNATE` uses the length set in the app. */
enum class Action(val command: String, val label: String, val short: String) {
    WINK_LH("WINK L", "Wink LH", "LH"),
    WINK_RH("WINK R", "Wink RH", "RH"),
    WINK_BOTH("WINK B", "Wink both", "Both"),
    ALTERNATE("ALT", "Alternate", "Alt"),
    STOP("STOP", "Stop", "Stop");

    companion object {
        fun fromName(name: String?): Action? = entries.firstOrNull { it.name == name }
    }
}

/** One value read from or notified on the state characteristic. */
sealed interface Notice {
    data class State(val state: String) : Notice
    data class Error(val reason: String, val restingSeconds: Int? = null) : Notice
    data class Remote(val status: String) : Notice
    data class Version(val version: String) : Notice
    data class DriveLock(val on: Boolean) : Notice
    data class Unknown(val text: String) : Notice
}

private val RESTING = Regex("""^resting (\d+)$""")

fun parseNotice(raw: ByteArray): Notice = parseNotice(String(raw, Charsets.US_ASCII))

fun parseNotice(raw: String): Notice {
    val t = raw.trim { it.isWhitespace() || it == '\u0000' }
    return when {
        t.startsWith("error:") -> {
            val reason = t.removePrefix("error:").trim()
            val m = RESTING.find(reason)
            if (m != null) Notice.Error("resting", m.groupValues[1].toInt()) else Notice.Error(reason)
        }
        t.startsWith("remote:") -> Notice.Remote(t.removePrefix("remote:").trim())
        t.startsWith("version:") -> Notice.Version(t.removePrefix("version:").trim())
        t == "drivelock: on" -> Notice.DriveLock(true)
        t == "drivelock: off" -> Notice.DriveLock(false)
        t in Protocol.STATES -> Notice.State(t)
        else -> Notice.Unknown(t)
    }
}

/** Short human text for a state word. */
fun describeState(state: String?): String = when (state) {
    null -> "No state yet."
    "asleep" -> "Module asleep. It wakes when a headlamp line changes (switch the headlamps)."
    "idle" -> "Ready."
    "wink" -> "Winking. Only Stop is accepted."
    "alternating" -> "Alternating. Only Stop is accepted."
    "resting" -> "Resting after alternating: winks work, alternate is locked for ${Protocol.REST_S} s."
    "overvoltage" -> "Supply too high on a module. Refused until it recovers."
    "undervoltage" -> "Supply too low for the relay to pull in (10-11.9 V, higher when hot). Refused until it recovers."
    else -> state
}

/** Short human text for a refusal. */
fun describeError(e: Notice.Error): String = when (e.reason) {
    "busy" -> "Refused: a wink or alternation is running. Only Stop works now."
    "resting" -> e.restingSeconds?.let { "Refused: alternate is locked for another $it s. Winks still work." }
        ?: "Refused: alternate is locked after each run. Winks still work."
    "overvoltage" -> "Refused: supply voltage too high."
    "undervoltage" -> "Refused: supply voltage too low."
    "engine running" -> "Refused: the engine is running and the drive lock is on."
    else -> "Refused: ${e.reason}."
}
