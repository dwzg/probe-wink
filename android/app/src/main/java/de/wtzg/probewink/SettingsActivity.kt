package de.wtzg.probewink

import android.app.Activity
import android.app.AlertDialog
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.LinearLayout
import android.widget.NumberPicker

/** All settings, grouped: modules and coupling, behaviour, remote, widgets, about, diagnostics. */
class SettingsActivity : Activity() {

    private lateinit var list: LinearLayout
    private val render: () -> Unit = { build() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.init(this)
        setContentView(R.layout.activity_list)
        UiKit.topBar(this, "Settings", back = true)
        list = findViewById(R.id.list)
    }

    override fun onStart() {
        super.onStart()
        Store.listen(render)
        build()
    }

    override fun onStop() {
        Store.unlisten(render)
        super.onStop()
    }

    private fun build() {
        val scrollY = (list.parent as? android.view.View)?.scrollY ?: 0
        list.removeAllViews()
        modules()
        behaviour()
        remote()
        widgets()
        about()
        (list.parent as? android.view.View)?.post { (list.parent as android.view.View).scrollTo(0, scrollY) }
    }

    // ---- modules and coupling
    private fun modules() {
        val card = UiKit.section(list, "Modules")
        UiKit.row(card, R.drawable.ic_bluetooth, "LH module",
            Store.address?.let { "${Store.deviceName ?: "Module"} · $it\nTakes all commands" } ?: "Not chosen. All commands go to this one.",
            chevron = true) { choose(Side.LH) }
        UiKit.row(card, R.drawable.ic_bluetooth, "RH module",
            Store.rhAddress?.let { "${Store.rhName ?: "Module"} · $it" } ?: "Not chosen. Needed once, for coupling.",
            chevron = true) { choose(Side.RH) }
        val fp = Store.linkFingerprint
        UiKit.row(card, R.drawable.ic_link, "Couple modules",
            when {
                fp == null -> "Status unknown. Connect once, or couple now."
                fp.isEmpty() -> "Not coupled. The RH lamp can't move yet."
                Store.rhSeen == false -> "Coupled (key $fp), RH not heard on the last check"
                else -> "Coupled (key $fp)"
            },
            chevron = true) { startActivity(Intent(this, CouplingActivity::class.java)) }
        if (fp != null && fp.isNotEmpty()) {
            UiKit.row(card, R.drawable.ic_link_off, "Uncouple", "Erase the link key on the LH module") {
                confirm("Uncouple the modules?", "The LH module erases its link key and restarts. The RH lamp stops following until you couple again.", "Uncouple") {
                    WinkService.run(this, "UNCOUPLE")
                }
            }
        }
    }

    @android.annotation.SuppressLint("MissingPermission")   // hasBluetooth() checked first
    private fun choose(side: Side) {
        if (!MainActivity.hasBluetooth(this)) {
            requestPermissions(MainActivity.neededPermissions().toTypedArray(), 1)
            return
        }
        ModulePicker.pick(this, side, onPicked = { d ->
            if (side == Side.LH) {
                Store.address = d.address
                Store.deviceName = d.name ?: "ProbeWink LH"
                Store.setProblem(null)
                if (d.bondState == android.bluetooth.BluetoothDevice.BOND_BONDED) WinkService.connect(this)
            } else {
                Store.rhAddress = d.address
                Store.rhName = d.name ?: "ProbeWink RH"
            }
            Store.log("${side.name} module set to ${d.address}")
            build()
        }, onPaired = { if (side == Side.LH) WinkService.connect(this) })
    }

    // ---- behaviour
    private fun behaviour() {
        val card = UiKit.section(list, "Behaviour")
        UiKit.row(card, R.drawable.ic_timer, "Alternate length", "${Store.altSeconds} s (at most ${Protocol.ALT_MAX_S} s)", chevron = true) {
            val picker = NumberPicker(this).apply {
                minValue = 1; maxValue = Protocol.ALT_MAX_S; value = Store.altSeconds
                wrapSelectorWheel = false
            }
            AlertDialog.Builder(this)
                .setTitle("Alternate length (seconds)")
                .setView(picker)
                .setPositiveButton("OK") { _, _ -> Store.altSeconds = picker.value; build(); Widgets.updateAll(this) }
                .setNegativeButton("Cancel", null)
                .show()
        }
        val lock = Store.driveLock
        UiKit.switchRow(card, R.drawable.ic_lock, "Drive lock",
            when (lock) {
                null -> "Refuse winks while the engine runs. Connect once to read the module's setting."
                true -> "On: winks and alternate are refused while the engine runs. Stop always works."
                false -> "Off. Turn on to refuse winks while the engine runs."
            },
            lock == true) { on ->
            if (Store.isSetUp) WinkService.run(this, Protocol.driveLock(on))
        }
        val keep = Store.keepAliveSeconds
        UiKit.row(card, R.drawable.ic_timer_hold, "Stay connected", "${label(keep)} after the last command", chevron = true) {
            val choices = Store.KEEP_ALIVE_CHOICES
            AlertDialog.Builder(this)
                .setTitle("Stay connected after a command")
                .setSingleChoiceItems(choices.map { label(it) }.toTypedArray(), choices.indexOf(keep).coerceAtLeast(0)) { d, i ->
                    Store.keepAliveSeconds = choices[i]
                    d.dismiss()
                    build()
                }
                .setNegativeButton("Cancel", null)
                .show()
        }
    }

    private fun label(s: Int) = if (s < 60 || s % 60 != 0) "$s s" else "${s / 60} min"

    // ---- remote
    private fun remote() {
        val card = UiKit.section(list, "Remote")
        UiKit.row(card, R.drawable.ic_bluetooth, "BLE remote", remoteSummary())
        UiKit.row(card, R.drawable.ic_link, "Pair a remote", "The LH module scans 30 s for a button remote", chevron = true) {
            confirm("Pair a remote", "Put the remote in pairing mode (or open the Flipper's Bluetooth Remote app), then tap Pair.", "Pair") {
                WinkService.run(this, "PAIR REMOTE")
            }
        }
        UiKit.row(card, R.drawable.ic_link_off, "Forget the remote") {
            confirm("Forget the remote?", "The LH module drops the stored remote and its bond.", "Forget") {
                WinkService.run(this, "FORGET REMOTE")
            }
        }
    }

    private fun remoteSummary() = when (Store.remote) {
        null -> "No report yet"
        "connected" -> "Connected"
        "disconnected" -> "Not connected (none paired, or asleep: press a button)"
        "forgotten" -> "None"
        else -> Store.remote!!.replaceFirstChar { it.uppercase() }
    } + "\nPresses: 1× wink LH · 2× wink RH · 3× both · long: alternate / stop"

    // ---- widgets
    private fun widgets() {
        val card = UiKit.section(list, "Widgets")
        val mgr = getSystemService(AppWidgetManager::class.java)
        UiKit.row(card, R.drawable.ic_widgets, "Add the control widget", "LH, RH, Both, Alt and Stop with the module state", chevron = true) {
            if (mgr.isRequestPinAppWidgetSupported) mgr.requestPinAppWidget(ComponentName(this, ControlWidget::class.java), null, null)
            else hint()
        }
        UiKit.row(card, R.drawable.ic_widgets, "Add a button widget", "One action per button; long-press it to change the action", chevron = true) {
            if (mgr.isRequestPinAppWidgetSupported) mgr.requestPinAppWidget(ComponentName(this, ActionWidget::class.java), null, null)
            else hint()
        }
    }

    private fun hint() =
        AlertDialog.Builder(this).setMessage(R.string.widget_hint).setPositiveButton("OK", null).show()

    // ---- about and diagnostics
    private fun about() {
        val card = UiKit.section(list, "About")
        val app = try { packageManager.getPackageInfo(packageName, 0).versionName } catch (_: Exception) { "?" }
        UiKit.row(card, R.drawable.ic_info, "Versions", "App $app · firmware ${Store.version ?: "unknown (connect once)"}")
        UiKit.row(card, R.drawable.ic_code, "Source code", "github.com/dwzg/probe-wink", chevron = true) {
            startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/dwzg/probe-wink")))
        }
        UiKit.row(card, R.drawable.ic_info, "Licences", "App and firmware MIT · hardware CERN-OHL-P-2.0 · icons Material Icons (Apache 2.0)")
        val diag = UiKit.section(list, "Diagnostics")
        UiKit.row(diag, R.drawable.ic_list, "Connection log", "Every command, reply and link event", chevron = true) {
            startActivity(Intent(this, LogActivity::class.java))
        }
    }

    private fun confirm(title: String, message: String, ok: String, action: () -> Unit) {
        AlertDialog.Builder(this)
            .setTitle(title)
            .setMessage(message)
            .setPositiveButton(ok) { _, _ -> action() }
            .setNegativeButton("Cancel", null)
            .show()
    }
}
