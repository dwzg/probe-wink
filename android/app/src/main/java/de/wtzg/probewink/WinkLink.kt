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
 * descriptor writes, command writes and key writes go through a queue. Writes report
 * their outcome through an optional callback.
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
        class Write(val command: String, val done: ((Boolean) -> Unit)?) : Op
        class WriteKey(val key: ByteArray, val done: (Boolean) -> Unit) : Op
    }

    private val main = Handler(Looper.getMainLooper())
    private var gatt: BluetoothGatt? = null
    private var command: BluetoothGattCharacteristic? = null
    private var state: BluetoothGattCharacteristic? = null
    private var keyChar: BluetoothGattCharacteristic? = null
    private val ops = ArrayDeque<Op>()
    private val pending = ArrayDeque<Op>()          // queued before the link is up
    private var current: Op? = null
    private var ready = false
    private var attempts = 0
    private var refreshed = false
    private var address: String? = null
    private val connectTimeout = Runnable { fail("Module not reachable. Is it awake and in range?") }

    val isReady get() = ready
    val connectedAddress get() = if (ready) address else null

    fun connect(address: String) {
        if (gatt != null && this.address == address) return
        close()
        this.address = address
        attempts = 0
        refreshed = false
        open()
    }

    /** Queue a command; it goes out once the link is ready. */
    fun send(cmd: String, done: ((Boolean) -> Unit)? = null) = queue(Op.Write(cmd, done))

    /** Queue a link-key write (coupling). Fails at once on firmware without the key characteristic. */
    fun writeKey(key: ByteArray, done: (Boolean) -> Unit) = queue(Op.WriteKey(key, done))

    private fun queue(op: Op) {
        if (ready) enqueue(op) else pending.addLast(op)
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
        keyChar = null
        failAll(ops)
        current?.let { finish(it, false) }
        current = null
        if (ready || address != null) events.onLink(Link.DISCONNECTED, null)
        ready = false
    }

    // ---- connection
    private fun open() {
        val addr = address ?: return
        val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
        if (adapter == null || !adapter.isEnabled) {
            events.onLink(Link.DISCONNECTED, "Bluetooth is off.")
            failAll(pending)
            return
        }
        val device = try { adapter.getRemoteDevice(addr) } catch (e: IllegalArgumentException) {
            events.onLink(Link.DISCONNECTED, "Bad module address $addr.")
            failAll(pending)
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
        failAll(pending)
        events.onLink(Link.DISCONNECTED, problem)
    }

    private fun failAll(q: ArrayDeque<Op>) {
        val dropped = q.toList()
        q.clear()
        dropped.forEach { finish(it, false) }
    }

    private fun finish(op: Op, ok: Boolean) {
        when (op) {
            is Op.Write -> op.done?.invoke(ok)
            is Op.WriteKey -> op.done(ok)
            else -> Unit
        }
    }

    private val callback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
            main.post {
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
                        failAll(ops)
                        current?.let { finish(it, false) }
                        current = null
                        main.removeCallbacks(connectTimeout)
                        // 133 (GATT_ERROR) right after connectGatt is common on Android: retry twice.
                        if (!wasReady && status == 133 && attempts < 3) {
                            Store.log("connect error 133, retrying")
                            main.postDelayed({ open() }, 600)
                        } else {
                            Store.log("disconnected (status $status)")
                            failAll(pending)
                            events.onLink(
                                Link.DISCONNECTED,
                                if (!wasReady && status != 0) "Connection failed (status $status)." else null,
                            )
                        }
                    }
                }
            }
        }

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
            main.post {
                if (g != gatt) return@post
                Store.log("MTU $mtu")
                g.discoverServices()
            }
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            main.post {
                if (g != gatt) return@post
                val svc = g.getService(Protocol.SERVICE)
                command = svc?.getCharacteristic(Protocol.COMMAND)
                state = svc?.getCharacteristic(Protocol.STATE)
                keyChar = svc?.getCharacteristic(Protocol.KEY)
                // Android caches a bonded device's services from the first connection. After a
                // firmware update that added characteristics, refresh the cache once and rediscover.
                if (svc != null && keyChar == null && !refreshed) {
                    refreshed = true
                    if (refreshCache(g)) {
                        Store.log("services look stale, refreshing Android's GATT cache")
                        main.postDelayed({ if (g == gatt) g.discoverServices() }, 300)
                        return@post
                    }
                }
                if (command == null || state == null) {
                    fail("This device has no ProbeWink service.")
                    return@post
                }
                // First access to a protected characteristic: Android pairs here if needed.
                enqueue(Op.ReadState)
                enqueue(Op.EnableNotify)
            }
        }

        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
            main.post { onRead(g, value, status) }
        }

        @Deprecated("API < 33")
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            if (Build.VERSION.SDK_INT >= 33) return
            @Suppress("DEPRECATION") val v = c.value ?: ByteArray(0)
            main.post { onRead(g, v, status) }
        }

        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
            main.post { if (g == gatt) notice(value) }
        }

        @Deprecated("API < 33")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            if (Build.VERSION.SDK_INT >= 33) return
            @Suppress("DEPRECATION") val v = c.value ?: ByteArray(0)
            main.post { if (g == gatt) notice(v) }
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
            main.post {
                if (g != gatt) return@post
                if (status != BluetoothGatt.GATT_SUCCESS) { authOrFail(status, "enable notifications"); return@post }
                done(true)
                ready = true
                attempts = 0
                main.removeCallbacks(connectTimeout)
                events.onLink(Link.CONNECTED, null)
                Store.log("ready")
                // Side, coupling, remote, version and drive lock come with STATUS.
                enqueue(Op.Write("STATUS", null))
                while (pending.isNotEmpty()) enqueue(pending.removeFirst())
            }
        }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            main.post {
                if (g != gatt) return@post
                if (status != BluetoothGatt.GATT_SUCCESS) {
                    current?.let { finish(it, false) }
                    current = null
                    authOrFail(status, "send a command")
                    return@post
                }
                done(true)
            }
        }
    }

    /** BluetoothGatt.refresh() is hidden API but stable for years; failure just means no refresh. */
    private fun refreshCache(g: BluetoothGatt): Boolean = try {
        g.javaClass.getMethod("refresh").invoke(g) as Boolean
    } catch (_: Exception) {
        false
    }

    private fun onRead(g: BluetoothGatt, value: ByteArray, status: Int) {
        if (g != gatt) return
        if (status != BluetoothGatt.GATT_SUCCESS) { authOrFail(status, "read the state"); return }
        notice(value)
        done(true)
    }

    private fun notice(value: ByteArray) {
        Store.log("← ${String(value, Charsets.US_ASCII).trimEnd('\u0000')}")
        events.onNotice(parseNotice(value))
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
            fail("The module refused the pairing. Remove it in Android's Bluetooth settings, then pair again.")
        }
    }

    // ---- operation queue
    private fun enqueue(op: Op) {
        ops.addLast(op)
        next()
    }

    private fun done(ok: Boolean) {
        current?.let { finish(it, ok) }
        current = null
        next()
    }

    private fun next() {
        if (current != null) return
        val g = gatt ?: return
        val op = ops.removeFirstOrNull() ?: return
        current = op
        val started = when (op) {
            Op.ReadState -> g.readCharacteristic(state)
            Op.EnableNotify -> {
                val c = state!!
                g.setCharacteristicNotification(c, true)
                val d = c.getDescriptor(Protocol.CCCD)
                d != null && writeDescriptor(g, d, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
            }
            is Op.Write -> {
                Store.log("→ ${op.command}")
                writeChar(g, command!!, op.command.toByteArray(Charsets.US_ASCII))
            }
            is Op.WriteKey -> {
                val k = keyChar
                if (k == null) {
                    Store.log("module firmware has no coupling support")
                    current = null
                    op.done(false)
                    next()
                    return
                }
                Store.log("→ link key (${op.key.size} bytes)")
                writeChar(g, k, op.key)
            }
        }
        if (!started) {
            // The stack refused to start the operation (usually busy): retry shortly.
            current = null
            ops.addFirst(op)
            main.postDelayed({ next() }, 150)
        }
    }

    private fun writeChar(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray): Boolean =
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
