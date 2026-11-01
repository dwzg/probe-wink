package de.wtzg.probewink

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

/**
 * Home-screen widgets. Buttons start [WinkService] directly (no activity), so one tap
 * winks even with the app closed.
 *
 * - [ControlWidget]: status line + LH / RH / Both / Alt / Stop.
 * - [ActionWidget]: one big button; the action is chosen when the widget is placed
 *   ([ActionWidgetConfigure]).
 */
object Widgets {
    fun updateAll(context: Context) {
        val mgr = AppWidgetManager.getInstance(context)
        mgr.getAppWidgetIds(ComponentName(context, ControlWidget::class.java)).forEach {
            mgr.updateAppWidget(it, ControlWidget.views(context))
        }
        mgr.getAppWidgetIds(ComponentName(context, ActionWidget::class.java)).forEach {
            mgr.updateAppWidget(it, ActionWidget.views(context, it))
        }
    }

    fun openApp(context: Context): PendingIntent = PendingIntent.getActivity(
        context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
    )

    fun statusText(): String = Store.problem ?: Store.lastReply ?: Store.shortStatus()
}

class ControlWidget : AppWidgetProvider() {
    companion object {
        private val BUTTONS = listOf(
            R.id.w_lh to Action.WINK_LH, R.id.w_rh to Action.WINK_RH, R.id.w_both to Action.WINK_BOTH,
            R.id.w_alt to Action.ALTERNATE, R.id.w_stop to Action.STOP,
        )

        fun views(context: Context): RemoteViews {
            Store.init(context)
            val v = RemoteViews(context.packageName, R.layout.widget_control)
            v.setTextViewText(R.id.w_status, Widgets.statusText())
            v.setOnClickPendingIntent(R.id.w_status, Widgets.openApp(context))
            for ((id, action) in BUTTONS) {
                v.setOnClickPendingIntent(id, WinkService.pending(context, action, 10 + action.ordinal))
            }
            return v
        }
    }

    override fun onUpdate(context: Context, mgr: AppWidgetManager, ids: IntArray) {
        ids.forEach { mgr.updateAppWidget(it, views(context)) }
    }
}

class ActionWidget : AppWidgetProvider() {
    companion object {
        fun views(context: Context, id: Int): RemoteViews {
            Store.init(context)
            val action = Store.widgetAction(id)
            val v = RemoteViews(context.packageName, R.layout.widget_action)
            v.setTextViewText(R.id.w_action, action.label)
            val stop = action == Action.STOP
            v.setInt(R.id.w_action, "setBackgroundResource", if (stop) R.drawable.btn_stop else R.drawable.btn_primary)
            v.setTextColor(R.id.w_action, context.getColor(if (stop) R.color.on_danger_soft else R.color.on_accent))
            // requestCode per widget: two widgets with different actions need distinct intents.
            v.setOnClickPendingIntent(R.id.w_action, WinkService.pending(context, action, 1000 + id))
            return v
        }
    }

    override fun onUpdate(context: Context, mgr: AppWidgetManager, ids: IntArray) {
        ids.forEach { mgr.updateAppWidget(it, views(context, it)) }
    }

    override fun onDeleted(context: Context, ids: IntArray) {
        Store.init(context)
        ids.forEach { Store.forgetWidget(it) }
    }
}
