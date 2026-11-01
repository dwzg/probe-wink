package de.wtzg.probewink

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Handler
import android.os.IBinder
import android.os.Looper

/**
 * Owns the Bluetooth link. Widgets, the app and the notification send it intents:
 * RUN (a command), CONNECT, DISCONNECT. It runs as a short-lived foreground service
 * (type connectedDevice): connect, send, stay connected [IDLE_MS] after the last
 * command so follow-up taps are instant, then disconnect and stop.
 *
 * Widget taps may start a foreground service from the background on Android 12+
 * (user interaction with an app widget is an allowed exemption).
 */
class WinkService : Service(), WinkLink.Events {

    companion object {
        const val ACTION_RUN = "de.wtzg.probewink.RUN"
        const val ACTION_CONNECT = "de.wtzg.probewink.CONNECT"
        const val ACTION_DISCONNECT = "de.wtzg.probewink.DISCONNECT"
        const val EXTRA_COMMAND = "command"
        private const val CHANNEL = "link"
        private const val NOTIFICATION_ID = 1
        private const val IDLE_MS = 90_000L

        fun intent(context: Context, action: String, command: String? = null): Intent =
            Intent(context, WinkService::class.java).setAction(action).apply {
                if (command != null) putExtra(EXTRA_COMMAND, command)
            }

        fun run(context: Context, command: String) =
            context.startForegroundService(intent(context, ACTION_RUN, command))

        fun connect(context: Context) = context.startForegroundService(intent(context, ACTION_CONNECT))

        fun disconnect(context: Context) = context.startService(intent(context, ACTION_DISCONNECT))

        /** PendingIntent for a widget button. requestCode keeps the intents distinct. */
        fun pending(context: Context, action: Action, requestCode: Int): PendingIntent =
            PendingIntent.getForegroundService(
                context, requestCode, intent(context, ACTION_RUN, action.name),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
    }

    private val main = Handler(Looper.getMainLooper())
    private lateinit var link: WinkLink
    private val idle = Runnable { stopNow("idle") }

    override fun onCreate() {
        super.onCreate()
        Store.init(this)
        link = WinkLink(this, this)
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.channel_link), NotificationManager.IMPORTANCE_LOW)
        )
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Must be in the foreground within seconds of startForegroundService().
        startForeground(NOTIFICATION_ID, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)

        if (intent?.action == ACTION_DISCONNECT) {
            stopNow("disconnect requested")
            return START_NOT_STICKY
        }
        if (checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED) {
            Store.setProblem("Open the app once and allow Nearby devices.")
            stopNow("no permission")
            return START_NOT_STICKY
        }
        val address = Store.address
        if (address == null) {
            Store.setProblem("Open the app and choose your module first.")
            stopNow("no module chosen")
            return START_NOT_STICKY
        }
        link.connect(address)
        if (intent?.action == ACTION_RUN) {
            val raw = intent.getStringExtra(EXTRA_COMMAND)
            val cmd = Action.fromName(raw)?.let { a ->
                if (a == Action.ALTERNATE) Protocol.alt(Store.altSeconds) else a.command
            } ?: raw
            if (cmd != null && cmd.length <= Protocol.MAX_LEN) {
                Store.clearReply()
                link.send(cmd)
            }
        }
        main.removeCallbacks(idle)
        main.postDelayed(idle, IDLE_MS)
        return START_NOT_STICKY
    }

    override fun onLink(link: Link, problem: String?) {
        Store.setLink(link, problem)
        if (link != Link.DISCONNECTED) {
            getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification())
        } else if (problem != null) {
            // A failed attempt: no point keeping the service around.
            main.removeCallbacks(idle)
            main.postDelayed({ stopNow("link failed") }, 500)
        }
    }

    override fun onNotice(notice: Notice) {
        Store.apply(notice)
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification())
    }

    private fun stopNow(why: String) {
        Store.log("service stopping: $why")
        main.removeCallbacks(idle)
        if (::link.isInitialized) link.close()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        main.removeCallbacks(idle)
        if (::link.isInitialized) link.close()
        super.onDestroy()
    }

    private fun notification(): Notification {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
        )
        val stop = PendingIntent.getForegroundService(
            this, 90, intent(this, ACTION_RUN, Action.STOP.name), PendingIntent.FLAG_IMMUTABLE,
        )
        val disconnect = PendingIntent.getService(
            this, 91, intent(this, ACTION_DISCONNECT), PendingIntent.FLAG_IMMUTABLE,
        )
        val text = when (Store.link) {
            Link.CONNECTING -> "Connecting…"
            Link.CONNECTED -> "Connected · ${Store.state ?: "…"}"
            Link.DISCONNECTED -> "Disconnected"
        }
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_wink)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .addAction(Notification.Action.Builder(null, "Disconnect", disconnect).build())
            .build()
    }
}
