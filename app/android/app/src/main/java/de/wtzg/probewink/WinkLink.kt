package de.wtzg.probewink

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper

/**
 * GATT client for one module. Everything runs on the main thread: Bluetooth callbacks
 * are posted over from the binder thread. GATT allows one operation at a time, so reads,
 * descriptor writes and command writes go through a queue.
 *
 * Permissions (BLUETOOTH_CONNECT) are checked by the caller.
 */
@SuppressLint("MissingPermission")
class WinkLink(private val context: Context, private val events: Events) {

    interface Events {
        fun onLink(link: Link, problem: String?)
        fun onNotice(notice: Notice)
    }

    private sealed interface Op {
        data object ReadState : Op
        data object EnableNotify : Op
        data class Write(val command: String) : Op
    }

    private val main = Handler(Looper.getMainLooper())
    private var gatt: BluetoothGatt? = null
    private var command: BluetoothGattCharacteristic? = null
    private var state: BluetoothGattCharacteristic? = null
    private val ops = ArrayDeque<Op>()
    private val pending = ArrayDeque<String>()      // commands queued before the link is up
    private var busy = false
    private var ready = false
    private var attempts = 0
    private var address: String? = null
    private val connectTimeout = Runnable { fail("Module not reachable. Is it awake and in range?") }

    val isReady get() = ready
    val isOpen get() = gatt != null

    fun connect(address: String) {
        if (gatt != null && this.address == address) return
        close()
        this.address = address
        attempts = 0
        open()
    }

    /** Queue a command; connects first if needed (the caller passes the address). */
    fun send(cmd: String) {
        if (ready) enqueue(Op.Write(cmd)) else pending.addLast(cmd)
    }

    fun close() {
        main.removeCallbacks(connectTimeout)
        gatt?.let {
            try { it.disconnect() } catch (_: Exception) {}
            try { it.close() } catch (_: Exception) {}
        }
        gatt = null
        command = null
        state = null
        ops.clear()
        busy = false
        if (ready || address != null) events.onLink(Link.DISCONNECTED, null)
        ready = false
    }

    // ---- connection
    private fun open() {
        val addr = address ?: return
        val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
        if (adapter == null || !adapter.isEnabled) {
            events.onLink(Link.DISCONNECTED, "Bluetooth is off.")
            return
        }
        val device = try { adapter.getRemoteDevice(addr) } catch (e: IllegalArgumentException) {
            events.onLink(Link.DISCONNECTED, "Bad module address $addr.")
            return
        }
        attempts++
        Store.log("connecting to $addr (attempt $attempts)")
        events.onLink(Link.CONNECTING, null)
        main.removeCallbacks(connectTimeout)
        main.postDelayed(connectTimeout, 15_000)
        gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
    }

    private fun fail(problem: String) {
        Store.log("link failed: $problem")
        val keepAddress = address
        close()
        address = keepAddress
        pending.clear()
        events.onLink(Link.DISCONNECTED, problem)
    }

    private val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) = main.post {
            if (g != gatt) return@post
            when {
                newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS -> {
                    Store.log("connected, asking for a larger MTU")
                    if (!g.requestMtu(185)) g.discoverServices()
                }
                newState == BluetoothProfile.STATE_DISCONNECTED -> {
                    val wasReady = ready
                    try { g.close() } catch (_: Exception) {}
                    gatt = null
                    ready = false
                    busy = false
                    ops.clear()
                    main.removeCallbacks(connectTimeout)
                    // 133 (GATT_ERROR) right after connectGatt is common on Android: retry twice.
                    if (!wasReady && status == 133 && attempts < 3) {
                        Store.log("connect error 133, retrying")
                        main.postDelayed({ open() }, 600)
                    } else {
                        Store.log("disconnected (status $status)")
                        events.onLink(Link.DISCONNECTED, if (!wasReady && status != 0) "Connection failed (status $status)." else null)
                    }
                }
            }
        }.let { }

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) = main.post {
            if (g != gatt) return@post
            Store.log("MTU $mtu")
            g.discoverServices()
        }.let { }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) = main.post {
            if (g != gatt) return@post
            val svc = g.getService(Protocol.SERVICE)
            command = svc?.getCharacteristic(Protocol.COMMAND)
            state = svc?.getCharacteristic(Protocol.STATE)
            if (command == null || state == null) {
                fail("This device has no ProbeWink service. Is it the LH (master) module?")
                return@post
            }
            // First access to a protected characteristic: Android pairs here if needed.
            enqueue(Op.ReadState)
            enqueue(Op.EnableNotify)
        }.let { }

        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) =
            main.post { onRead(g, value, status) }.let { }

        @Deprecated("API < 33")
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            if (Build.VERSION.SDK_INT >= 33) return
            @Suppress("DEPRECATION") val v = c.value ?: ByteArray(0)
            main.post { onRead(g, v, status) }
        }

        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) =
            main.post { if (g == gatt) notice(value) }.let { }

        @Deprecated("API < 33")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            if (Build.VERSION.SDK_INT >= 33) return
            @Suppress("DEPRECATION") val v = c.value ?: ByteArray(0)
            main.post { if (g == gatt) notice(v) }
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) = main.post {
            if (g != gatt) return@post
            if (status != BluetoothGatt.GATT_SUCCESS) { authOrFail(status, "enable notifications"); return@post }
            done()
            ready = true
            attempts = 0
            main.removeCallbacks(connectTimeout)
            events.onLink(Link.CONNECTED, null)
            Store.log("ready")
            // Remote link state and firmware version come with STATUS.
            enqueue(Op.Write("STATUS"))
            while (pending.isNotEmpty()) enqueue(Op.Write(pending.removeFirst()))
        }.let { }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) = main.post {
            if (g != gatt) return@post
            if (status != BluetoothGatt.GATT_SUCCESS) { authOrFail(status, "send a command"); return@post }
            done()
        }.let { }
    }

    private fun onRead(g: BluetoothGatt, value: ByteArray, status: Int) {
        if (g != gatt) return
        if (status != BluetoothGatt.GATT_SUCCESS) { authOrFail(status, "read the state"); return }
        notice(value)
        done()
    }

    private fun notice(value: ByteArray) {
        val n = parseNotice(value)
        Store.log("← ${String(value, Charsets.US_ASCII).trimEnd('\u0000')}")
        events.onNotice(n)
    }

    /** 5 / 15 / 137: the link is not authenticated (no bond, or a stale bond on one side). */
    private fun authOrFail(status: Int, what: String) {
        val auth = status == 5 || status == 15 || status == 137
        if (!auth) { fail("Could not $what (GATT status $status)."); return }
        val device = gatt?.device
        if (device != null && device.bondState != BluetoothDevice.BOND_BONDED) {
            Store.log("not paired yet: starting pairing (enter the passkey when Android asks)")
            device.createBond()
            fail("Pairing started: enter the module's passkey when Android asks, then try again.")
        } else {
            fail("The module refused the pairing. Remove PWink in Android's Bluetooth settings, then pair again from this app.")
        }
    }

    // ---- operation queue
    private fun enqueue(op: Op) {
        ops.addLast(op)
        next()
    }

    private fun done() {
        busy = false
        next()
    }

    private fun next() {
        if (busy) return
        val g = gatt ?: return
        val op = ops.removeFirstOrNull() ?: return
        busy = true
        val ok = when (op) {
            Op.ReadState -> g.readCharacteristic(state)
            Op.EnableNotify -> {
                val c = state!!
                g.setCharacteristicNotification(c, true)
                val d = c.getDescriptor(Protocol.CCCD)
                if (d == null) false else writeDescriptor(g, d, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
            }
            is Op.Write -> {
                Store.log("→ ${op.command}")
                writeCommand(g, command!!, op.command.toByteArray(Charsets.US_ASCII))
            }
        }
        if (!ok) {
            busy = false
            // The stack refused to start the operation (usually busy): retry shortly.
            ops.addFirst(op)
            main.postDelayed({ next() }, 150)
        }
    }

    private fun writeCommand(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray): Boolean =
        if (Build.VERSION.SDK_INT >= 33) {
            g.writeCharacteristic(c, value, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT) == BluetoothStatusCodes.SUCCESS
        } else {
            @Suppress("DEPRECATION")
            c.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
            @Suppress("DEPRECATION")
            c.value = value
            @Suppress("DEPRECATION")
            g.writeCharacteristic(c)
        }

    private fun writeDescriptor(g: BluetoothGatt, d: BluetoothGattDescriptor, value: ByteArray): Boolean =
        if (Build.VERSION.SDK_INT >= 33) {
            g.writeDescriptor(d, value) == BluetoothStatusCodes.SUCCESS
        } else {
            @Suppress("DEPRECATION")
            d.value = value
            @Suppress("DEPRECATION")
            g.writeDescriptor(d)
        }
}
