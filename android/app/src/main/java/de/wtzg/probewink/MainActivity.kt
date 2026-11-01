package de.wtzg.probewink

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
import android.appwidget.AppWidgetManager
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import android.view.View
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.Switch
import android.widget.TextView

/** Setup and full control: module choice, all commands, remote pairing, state and log. */
@SuppressLint("MissingPermission")   // every Bluetooth call is behind hasBluetooth()
class MainActivity : Activity() {

    private val main = Handler(Looper.getMainLooper())
    private val render: () -> Unit = { render() }
    private var logOpen = false

    /** Pairing with a newly chosen module finished: connect right away. */
    private val bonds = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val d = (if (Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
            } else {
                @Suppress("DEPRECATION") intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE)
            }) ?: return
            val s = intent.getIntExtra(BluetoothDevice.EXTRA_BOND_STATE, BluetoothDevice.ERROR)
            if (d.address != Store.address) return
            when (s) {
                BluetoothDevice.BOND_BONDED -> { Store.log("paired with ${d.address}"); WinkService.connect(this@MainActivity) }
                BluetoothDevice.BOND_NONE -> Store.setProblem("Pairing failed or was cancelled. Check the passkey and try Choose again.")
            }
        }
    }

    private lateinit var state: TextView
    private lateinit var stateText: TextView
    private lateinit var reply: TextView
    private lateinit var remote: TextView
    private lateinit var module: TextView
    private lateinit var connect: Button
    private lateinit var altSecs: TextView
    private lateinit var log: TextView
    private lateinit var logBox: View
    private lateinit var commandButtons: List<Button>
    private lateinit var driveLock: Switch

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.init(this)
        setContentView(R.layout.activity_main)
        state = findViewById(R.id.state)
        stateText = findViewById(R.id.state_text)
        reply = findViewById(R.id.reply)
        remote = findViewById(R.id.remote)
        module = findViewById(R.id.module)
        connect = findViewById(R.id.btn_connect)
        altSecs = findViewById(R.id.alt_secs)
        log = findViewById(R.id.log)
        logBox = findViewById(R.id.log_box)

        val cmd = mapOf(
            R.id.btn_lh to Action.WINK_LH, R.id.btn_rh to Action.WINK_RH, R.id.btn_both to Action.WINK_BOTH,
            R.id.btn_alt to Action.ALTERNATE, R.id.btn_stop to Action.STOP,
        )
        commandButtons = cmd.map { (id, action) ->
            findViewById<Button>(id).apply { setOnClickListener { send(action.name) } }
        }
        findViewById<Button>(R.id.btn_alt_minus).setOnClickListener { Store.altSeconds -= 1; render() }
        findViewById<Button>(R.id.btn_alt_plus).setOnClickListener { Store.altSeconds += 1; render() }
        findViewById<Button>(R.id.btn_choose).setOnClickListener { chooseModule() }
        connect.setOnClickListener {
            if (Store.link == Link.DISCONNECTED) WinkService.connect(this) else WinkService.disconnect(this)
        }
        findViewById<Button>(R.id.btn_status).setOnClickListener { send("STATUS") }
        findViewById<Button>(R.id.btn_pair).setOnClickListener {
            AlertDialog.Builder(this)
                .setTitle("Pair a remote")
                .setMessage("Put the remote in pairing mode (or open the Flipper's Bluetooth Remote app), then tap Pair. The module scans for 30 s.")
                .setPositiveButton("Pair") { _, _ -> send("PAIR REMOTE") }
                .setNegativeButton("Cancel", null).show()
        }
        findViewById<Button>(R.id.btn_forget).setOnClickListener {
            AlertDialog.Builder(this)
                .setTitle("Forget the remote?")
                .setMessage("The module drops the stored remote and its bond.")
                .setPositiveButton("Forget") { _, _ -> send("FORGET REMOTE") }
                .setNegativeButton("Cancel", null).show()
        }
        findViewById<Button>(R.id.btn_widget).setOnClickListener { pinWidget() }
        driveLock = findViewById(R.id.drive_lock)
        findViewById<Button>(R.id.btn_log).setOnClickListener { logOpen = !logOpen; render() }
    }

    override fun onStart() {
        super.onStart()
        Store.listen(render)
        // A system broadcast: needs to be exported (flag required from Android 14).
        val filter = IntentFilter(BluetoothDevice.ACTION_BOND_STATE_CHANGED)
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(bonds, filter, RECEIVER_EXPORTED) else registerReceiver(bonds, filter)
        askPermissions()
        render()
    }

    override fun onStop() {
        Store.unlisten(render)
        unregisterReceiver(bonds)
        super.onStop()
    }

    // ---- actions
    private fun send(command: String) {
        if (!hasBluetooth()) { askPermissions(); return }
        if (Store.address == null) { chooseModule(); return }
        WinkService.run(this, command)
    }

    private fun pinWidget() {
        val mgr = getSystemService(AppWidgetManager::class.java)
        if (mgr.isRequestPinAppWidgetSupported) {
            mgr.requestPinAppWidget(ComponentName(this, ControlWidget::class.java), null, null)
        } else {
            AlertDialog.Builder(this).setMessage(R.string.widget_hint).setPositiveButton("OK", null).show()
        }
    }

    // ---- module choice: paired modules first, then a scan
    private fun chooseModule() {
        if (!hasBluetooth()) { askPermissions(); return }
        val adapter = getSystemService(BluetoothManager::class.java)?.adapter
        if (adapter == null || !adapter.isEnabled) {
            AlertDialog.Builder(this).setMessage("Turn Bluetooth on first.").setPositiveButton("OK", null).show()
            return
        }
        val bonded = adapter.bondedDevices.filter { it.name in Protocol.NAMES }.sortedBy { it.name }
        val labels = bonded.map { "${it.name}  ${it.address}" } + "Scan for modules…"
        AlertDialog.Builder(this)
            .setTitle("Choose the LH (master) module")
            .setItems(labels.toTypedArray()) { _, i ->
                if (i < bonded.size) use(bonded[i]) else scan()
            }
            .setNegativeButton("Cancel", null).show()
    }

    private fun scan() {
        val scanner = getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeScanner ?: return
        val found = linkedMapOf<String, BluetoothDevice>()
        val items = ArrayAdapter<String>(this, android.R.layout.simple_list_item_1)
        lateinit var dialog: AlertDialog
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val d = result.device
                if (found.put(d.address, d) == null) {
                    items.add("${result.scanRecord?.deviceName ?: d.name ?: "?"}  ${d.address}  ${result.rssi} dBm")
                }
            }
        }
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(Protocol.SERVICE)).build()
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        dialog = AlertDialog.Builder(this)
            .setTitle("Scanning… (make sure the module is awake)")
            .setAdapter(items) { _, i ->
                scanner.stopScan(cb)
                use(found.values.elementAt(i))
            }
            .setNegativeButton("Cancel") { _, _ -> scanner.stopScan(cb) }
            .setOnDismissListener { try { scanner.stopScan(cb) } catch (_: Exception) {} }
            .show()
        scanner.startScan(listOf(filter), settings, cb)
        main.postDelayed({
            try { scanner.stopScan(cb) } catch (_: Exception) {}
            if (dialog.isShowing && found.isEmpty()) dialog.setTitle("No module found. Is it awake and in range?")
        }, 12_000)
    }

    private fun use(device: BluetoothDevice) {
        Store.address = device.address
        Store.deviceName = device.name ?: "PWink"
        Store.setProblem(null)
        Store.log("module set to ${device.address}")
        if (device.bondState != BluetoothDevice.BOND_BONDED) {
            Store.log("pairing: enter the module's passkey when Android asks")
            device.createBond()
        } else {
            WinkService.connect(this)
        }
        render()
    }

    // ---- permissions
    private fun needed(): List<String> = buildList {
        add(Manifest.permission.BLUETOOTH_CONNECT)
        add(Manifest.permission.BLUETOOTH_SCAN)
        if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
    }

    private fun hasBluetooth() =
        checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED &&
            checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN) == PackageManager.PERMISSION_GRANTED

    private fun askPermissions() {
        val missing = needed().filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 1)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (!hasBluetooth()) Store.setProblem("Allow Nearby devices, or the app can't reach the module.")
        else if (Store.problem?.startsWith("Allow Nearby") == true || Store.problem?.startsWith("Open the app once") == true) Store.setProblem(null)
        render()
    }

    // ---- view
    private fun render() {
        val s = Store.state
        state.text = s ?: "—"
        state.setTextColor(getColor(when (s) {
            "overvoltage", "undervoltage" -> R.color.fault
            "wink", "alternating" -> R.color.accent
            else -> R.color.text
        }))
        stateText.text = describeState(s)
        val r = Store.problem ?: Store.lastReply
        reply.visibility = if (r == null) View.GONE else View.VISIBLE
        reply.text = r
        remote.text = buildString {
            append("Remote: ").append(Store.remote ?: "no report yet")
            Store.version?.let { append("  ·  firmware ").append(it) }
        }
        module.text = Store.address?.let { "${Store.deviceName ?: "Module"}  $it" } ?: "No module chosen"
        connect.text = when (Store.link) {
            Link.DISCONNECTED -> "Connect"
            Link.CONNECTING -> "Connecting…"
            Link.CONNECTED -> "Disconnect"
        }
        connect.isEnabled = Store.address != null
        altSecs.text = "${Store.altSeconds} s"
        findViewById<Button>(R.id.btn_alt).text = "Alternate ${Store.altSeconds} s"
        val running = s == "wink" || s == "alternating"
        for (b in commandButtons) {
            b.isEnabled = Store.address != null && (b.id == R.id.btn_stop || !running || Store.link != Link.CONNECTED)
        }
        findViewById<Button>(R.id.btn_alt).isEnabled = Store.address != null && !(Store.link == Link.CONNECTED && (running || s == "resting"))
        // Drive lock: shows the module's stored setting; tapping sends the change.
        driveLock.setOnCheckedChangeListener(null)
        driveLock.isChecked = Store.driveLock == true
        driveLock.isEnabled = Store.address != null
        driveLock.text = if (Store.driveLock == null) "Block winks while the engine runs (connect to read)"
            else "Block winks while the engine runs"
        driveLock.setOnCheckedChangeListener { _, on -> send(Protocol.driveLock(on)) }
        logBox.visibility = if (logOpen) View.VISIBLE else View.GONE
        findViewById<Button>(R.id.btn_log).text = if (logOpen) "Hide log" else "Show log"
        if (logOpen) log.text = Store.logText(last = 60)
    }
}
