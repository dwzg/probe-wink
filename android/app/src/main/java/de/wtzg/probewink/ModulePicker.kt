package de.wtzg.probewink

import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import android.widget.ArrayAdapter

/**
 * Choose a module for one side: paired modules first, then a scan for the ProbeWink
 * service. A module that isn't paired yet gets paired (Android asks for the passkey);
 * [onPaired] runs once the bond exists.
 */
@SuppressLint("MissingPermission")   // callers check BLUETOOTH_CONNECT / SCAN first
object ModulePicker {

    fun pick(a: Activity, side: Side, onPicked: (BluetoothDevice) -> Unit, onPaired: (BluetoothDevice) -> Unit = {}) {
        val adapter = a.getSystemService(BluetoothManager::class.java)?.adapter
        if (adapter == null || !adapter.isEnabled) {
            AlertDialog.Builder(a).setMessage("Turn Bluetooth on first.").setPositiveButton("OK", null).show()
            return
        }
        val bonded = adapter.bondedDevices
            .filter { Protocol.isModuleName(it.name) && Protocol.sideOf(it.name).let { s -> s == null || s == side } }
            .sortedBy { it.name }
        val labels = bonded.map { "${it.name}   ${it.address}" } + "Scan for modules…"
        AlertDialog.Builder(a)
            .setTitle(if (side == Side.LH) "LH module" else "RH module")
            .setItems(labels.toTypedArray()) { _, i ->
                if (i < bonded.size) onPicked(bonded[i]) else scan(a, side, onPicked, onPaired)
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun scan(a: Activity, side: Side, onPicked: (BluetoothDevice) -> Unit, onPaired: (BluetoothDevice) -> Unit) {
        val scanner = a.getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeScanner ?: return
        val main = Handler(Looper.getMainLooper())
        val found = linkedMapOf<String, BluetoothDevice>()
        val items = ArrayAdapter<String>(a, android.R.layout.simple_list_item_1)
        lateinit var dialog: AlertDialog
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val d = result.device
                val name = result.scanRecord?.deviceName ?: d.name
                val s = Protocol.sideOf(name)
                if (s != null && s != side) return
                if (found.put(d.address, d) == null) items.add("${name ?: "ProbeWink"}   ${d.address}   ${result.rssi} dBm")
            }
        }
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(Protocol.SERVICE)).build()
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        fun stop() = try { scanner.stopScan(cb) } catch (_: Exception) {}
        dialog = AlertDialog.Builder(a)
            .setTitle("Scanning… switch the headlamps once to wake the modules")
            .setAdapter(items) { _, i ->
                stop()
                val d = found.values.elementAt(i)
                onPicked(d)
                if (d.bondState != BluetoothDevice.BOND_BONDED) pair(a, d, onPaired)
            }
            .setNegativeButton("Cancel") { _, _ -> stop() }
            .setOnDismissListener { stop() }
            .show()
        scanner.startScan(listOf(filter), settings, cb)
        main.postDelayed({
            stop()
            if (dialog.isShowing && found.isEmpty()) dialog.setTitle("No module found. Is it awake and in range?")
        }, 12_000)
    }

    /** Start pairing and call [onPaired] when Android reports the bond. */
    fun pair(context: Context, device: BluetoothDevice, onPaired: (BluetoothDevice) -> Unit) {
        val app = context.applicationContext
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, intent: Intent) {
                val d = (if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
                } else {
                    @Suppress("DEPRECATION") intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE)
                }) ?: return
                if (d.address != device.address) return
                when (intent.getIntExtra(BluetoothDevice.EXTRA_BOND_STATE, BluetoothDevice.ERROR)) {
                    BluetoothDevice.BOND_BONDED -> {
                        Store.log("paired with ${d.address}")
                        app.unregisterReceiver(this)
                        onPaired(d)
                    }
                    BluetoothDevice.BOND_NONE -> {
                        Store.log("pairing with ${d.address} failed or was cancelled")
                        app.unregisterReceiver(this)
                    }
                }
            }
        }
        val filter = IntentFilter(BluetoothDevice.ACTION_BOND_STATE_CHANGED)
        if (Build.VERSION.SDK_INT >= 33) app.registerReceiver(receiver, filter, Context.RECEIVER_EXPORTED)
        else app.registerReceiver(receiver, filter)
        Store.log("pairing with ${device.address}: enter the module's passkey when Android asks")
        device.createBond()
    }
}
