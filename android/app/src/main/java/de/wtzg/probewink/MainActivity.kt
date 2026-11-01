package de.wtzg.probewink

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.View
import android.widget.ImageView
import android.widget.TextView
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** Control screen: module state and the five actions. Setup lives in [SettingsActivity]. */
class MainActivity : Activity() {

    private val render: () -> Unit = { render() }
    private lateinit var tiles: Map<Action, View>

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.init(this)
        setContentView(R.layout.activity_main)
        UiKit.topBar(
            this, getString(R.string.app_name), back = false,
            action = R.drawable.ic_settings to { startActivity(Intent(this, SettingsActivity::class.java)) },
            actionDesc = "Settings",
        )
        tiles = mapOf(
            Action.WINK_LH to findViewById(R.id.act_lh), Action.WINK_RH to findViewById(R.id.act_rh),
            Action.WINK_BOTH to findViewById(R.id.act_both), Action.ALTERNATE to findViewById(R.id.act_alt),
            Action.STOP to findViewById(R.id.act_stop),
        )
        tiles.forEach { (action, v) -> v.setOnClickListener { send(action) } }
        findViewById<View>(R.id.setup_start).setOnClickListener {
            startActivity(Intent(this, SettingsActivity::class.java))
        }
        findViewById<View>(R.id.conn_button).setOnClickListener {
            if (Store.link == Link.DISCONNECTED) WinkService.connect(this) else WinkService.disconnect(this)
        }
    }

    override fun onStart() {
        super.onStart()
        Store.listen(render)
        askPermissions()
        render()
    }

    override fun onStop() {
        Store.unlisten(render)
        super.onStop()
    }

    private fun send(action: Action) {
        if (!hasBluetooth(this)) { askPermissions(); return }
        if (!Store.isSetUp) { startActivity(Intent(this, SettingsActivity::class.java)); return }
        WinkService.run(this, action.name)
    }

    private fun askPermissions() {
        val missing = neededPermissions().filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 1)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (!hasBluetooth(this)) Store.setProblem("Allow Nearby devices, or the app can't reach the modules.")
        else if (Store.problem?.startsWith("Allow Nearby") == true) Store.setProblem(null)
    }

    private fun render() {
        val setUp = Store.isSetUp
        findViewById<View>(R.id.setup_card).visibility = if (setUp) View.GONE else View.VISIBLE
        findViewById<View>(R.id.status_card).visibility = if (setUp) View.VISIBLE else View.GONE

        // connection line
        val link = Store.link
        findViewById<ImageView>(R.id.conn_dot).imageTintList = getColorStateList(
            when (link) { Link.CONNECTED -> R.color.ok; Link.CONNECTING -> R.color.warn; Link.DISCONNECTED -> R.color.outline },
        )
        findViewById<TextView>(R.id.conn_text).text = when (link) {
            Link.CONNECTED -> "Connected to ${Store.deviceName ?: "LH module"}"
            Link.CONNECTING -> "Connecting…"
            Link.DISCONNECTED -> lastSeen()?.let { "Not connected · last seen $it" } ?: "Not connected"
        }
        findViewById<TextView>(R.id.conn_button).text = if (link == Link.DISCONNECTED) "Connect" else "Disconnect"

        // state
        val s = Store.state
        findViewById<TextView>(R.id.state).apply {
            text = s?.replaceFirstChar { it.uppercase() } ?: "Unknown"
            setTextColor(getColor(when (s) {
                "overvoltage", "undervoltage" -> R.color.danger
                "wink", "alternating" -> R.color.accent
                else -> R.color.text
            }))
        }
        findViewById<TextView>(R.id.state_text).text =
            if (s == null) "Connect once to read the module's state." else describeState(s)
        val r = Store.problem ?: Store.lastReply
        findViewById<TextView>(R.id.reply).apply {
            visibility = if (r == null) View.GONE else View.VISIBLE
            text = r
        }
        findViewById<TextView>(R.id.lh_status).text = when {
            link == Link.CONNECTED -> "Connected"
            lastSeen() != null -> "Seen ${lastSeen()}"
            else -> "Not reached yet"
        }
        val fp = Store.linkFingerprint
        val coupled = fp != null && fp.isNotEmpty()
        findViewById<TextView>(R.id.rh_status).text = when {
            fp == null -> "Unknown"
            !coupled -> "Not coupled"
            Store.rhSeen == true -> "Coupled, in range"
            Store.rhSeen == false -> "Coupled, not heard"
            else -> "Coupled"
        }

        // actions
        val running = s == "wink" || s == "alternating"
        val live = link == Link.CONNECTED
        for ((action, v) in tiles) {
            val needsRh = action == Action.WINK_RH || action == Action.WINK_BOTH || action == Action.ALTERNATE
            val enabled = setUp && (action == Action.STOP || !(live && running)) &&
                !(needsRh && fp == "") && !(action == Action.ALTERNATE && live && s == "resting")
            v.isEnabled = enabled
            v.alpha = if (enabled) 1f else 0.45f
        }
        findViewById<TextView>(R.id.act_alt_sub).text = when {
            fp == "" -> "Not coupled"
            s == "resting" -> "Resting"
            else -> "${Store.altSeconds} s"
        }
        if (fp == "") {
            findViewById<TextView>(R.id.act_rh_sub).text = "Not coupled"
            findViewById<TextView>(R.id.act_both_sub).text = "Not coupled"
        } else {
            findViewById<TextView>(R.id.act_rh_sub).text = "Right lamp"
            findViewById<TextView>(R.id.act_both_sub).text = "Flip and return"
        }

        findViewById<TextView>(R.id.footer).text = buildList {
            Store.remote?.let { add("Remote $it") }
            if (Store.driveLock == true) add("Drive lock on")
            Store.version?.let { add("Firmware $it") }
        }.joinToString("  ·  ")
    }

    private fun lastSeen(): String? =
        Store.stateAt.takeIf { it > 0 }?.let { SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(it)) }

    companion object {
        fun neededPermissions(): List<String> = buildList {
            add(Manifest.permission.BLUETOOTH_CONNECT)
            add(Manifest.permission.BLUETOOTH_SCAN)
            if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
        }

        fun hasBluetooth(a: Activity) =
            a.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED &&
                a.checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN) == PackageManager.PERMISSION_GRANTED
    }
}
