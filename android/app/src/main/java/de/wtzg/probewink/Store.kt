package de.wtzg.probewink

import android.content.Context
import android.content.SharedPreferences
import android.os.Handler
import android.os.Looper
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

enum class Link { DISCONNECTED, CONNECTING, CONNECTED }

/**
 * Settings and the last known module state, shared by the activities, the service and
 * the widgets (all in one process). Persisted so widgets show something after a restart.
 * Change listeners are called on the main thread.
 */
object Store {
    private const val PREFS = "probewink"
    private lateinit var prefs: SharedPreferences
    private val main = Handler(Looper.getMainLooper())
    private val listeners = mutableSetOf<() -> Unit>()
    private val logLines = ArrayDeque<String>()
    private val clock = SimpleDateFormat("HH:mm:ss", Locale.ROOT)
    private var appContext: Context? = null

    val KEEP_ALIVE_CHOICES = listOf(30, 90, 300)

    fun init(context: Context) {
        if (appContext != null) return
        appContext = context.applicationContext
        prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    }

    // ---- modules
    /** The LH (master) module: all commands go here. */
    var address: String?
        get() = prefs.getString("address", null)
        set(v) = edit { putString("address", v) }
    var deviceName: String?
        get() = prefs.getString("device_name", null)
        set(v) = edit { putString("device_name", v) }
    /** The RH module: only needed for coupling. */
    var rhAddress: String?
        get() = prefs.getString("rh_address", null)
        set(v) = edit { putString("rh_address", v) }
    var rhName: String?
        get() = prefs.getString("rh_name", null)
        set(v) = edit { putString("rh_name", v) }

    // ---- settings
    var altSeconds: Int
        get() = prefs.getInt("alt_seconds", Protocol.ALT_DEFAULT_S)
        set(v) = edit { putInt("alt_seconds", v.coerceIn(1, Protocol.ALT_MAX_S)) }
    /** How long the service stays connected after the last command. */
    var keepAliveSeconds: Int
        get() = prefs.getInt("keep_alive_s", 90)
        set(v) = edit { putInt("keep_alive_s", v) }

    fun widgetAction(id: Int): Action = Action.fromName(prefs.getString("widget_$id", null)) ?: Action.WINK_BOTH
    fun setWidgetAction(id: Int, a: Action) = edit { putString("widget_$id", a.name) }
    fun forgetWidget(id: Int) = edit { remove("widget_$id") }

    // ---- live state of the LH module (persisted for the widgets)
    val link: Link get() = Link.entries.getOrElse(prefs.getInt("link", 0)) { Link.DISCONNECTED }
    val state: String? get() = prefs.getString("state", null)
    val stateAt: Long get() = prefs.getLong("state_at", 0)
    val remote: String? get() = prefs.getString("remote", null)
    val version: String? get() = prefs.getString("version", null)
    val lastReply: String? get() = prefs.getString("reply", null)
    val problem: String? get() = prefs.getString("problem", null)
    /** Drive lock as last reported by the module; null = not known yet. */
    val driveLock: Boolean? get() = if (prefs.contains("drive_lock")) prefs.getBoolean("drive_lock", false) else null
    /** Coupling as last reported by the LH module: null = unknown, "" = not coupled, else fingerprint. */
    val linkFingerprint: String? get() = prefs.getString("link_fp", null)
    /** Did the LH module hear the RH module recently (last STATUS)? */
    val rhSeen: Boolean? get() = if (prefs.contains("rh_seen")) prefs.getBoolean("rh_seen", false) else null

    val isSetUp: Boolean get() = address != null

    fun setLink(l: Link, problem: String? = null) {
        edit {
            putInt("link", l.ordinal)
            if (problem != null) putString("problem", problem) else if (l == Link.CONNECTED) remove("problem")
        }
        changed()
    }

    fun setProblem(p: String?) {
        edit { if (p == null) remove("problem") else putString("problem", p) }
        changed()
    }

    /** A notice from the LH module (the service's link). */
    fun apply(n: Notice) {
        when (n) {
            is Notice.State -> edit { putString("state", n.state); putLong("state_at", System.currentTimeMillis()) }
            is Notice.Error -> edit { putString("reply", describeError(n)) }
            is Notice.Remote -> edit { putString("remote", n.status) }
            is Notice.Version -> edit { putString("version", n.version) }
            is Notice.DriveLock -> edit { putBoolean("drive_lock", n.on) }
            is Notice.Link -> edit {
                putString("link_fp", n.fingerprint ?: "")
                if (n.rhSeen != null) putBoolean("rh_seen", n.rhSeen) else remove("rh_seen")
            }
            is Notice.SideIs, is Notice.Unknown -> Unit
        }
        changed()
    }

    fun clearReply() = edit { remove("reply") }

    /** After coupling: what the LH module will report. */
    fun setCoupled(fingerprint: String?) {
        edit { putString("link_fp", fingerprint ?: ""); remove("rh_seen") }
        changed()
    }

    // ---- log (memory only)
    fun log(line: String) {
        val stamped = "${clock.format(Date())}  $line"
        main.post {
            logLines.addLast(stamped)
            while (logLines.size > 500) logLines.removeFirst()
            changed()
        }
    }

    /** Newest lines last. */
    fun logText(last: Int = 500): String = logLines.toList().takeLast(last).joinToString("\n")

    fun clearLog() {
        logLines.clear()
        changed()
    }

    // ---- change notification
    fun listen(l: () -> Unit) = listeners.add(l)
    fun unlisten(l: () -> Unit) = listeners.remove(l)

    private fun changed() {
        main.post {
            listeners.toList().forEach { it() }
            appContext?.let { Widgets.updateAll(it) }
        }
    }

    private inline fun edit(block: SharedPreferences.Editor.() -> Unit) {
        prefs.edit().apply(block).apply()
    }

    /** "idle · 14:32" style status for widgets. */
    fun shortStatus(): String {
        val s = state ?: return if (address == null) "Tap to set up" else "No state yet"
        val t = if (stateAt > 0) SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(stateAt)) else ""
        val linkMark = if (link == Link.CONNECTING) " · connecting" else ""
        return "$s · $t$linkMark"
    }
}
