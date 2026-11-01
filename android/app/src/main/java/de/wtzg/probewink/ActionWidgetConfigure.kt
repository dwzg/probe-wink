package de.wtzg.probewink

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import android.widget.LinearLayout

/** Shown when a button widget is placed or reconfigured: pick its action. */
class ActionWidgetConfigure : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.init(this)
        val id = intent?.extras?.getInt(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
            ?: AppWidgetManager.INVALID_APPWIDGET_ID
        setResult(RESULT_CANCELED, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id))
        if (id == AppWidgetManager.INVALID_APPWIDGET_ID) { finish(); return }

        setContentView(R.layout.activity_list)
        UiKit.topBar(this, "Button widget", back = true)
        val list = findViewById<LinearLayout>(R.id.list)
        val card = UiKit.section(list, "What should this button do?")
        val icons = mapOf(
            Action.WINK_LH to R.drawable.ic_wink_lh, Action.WINK_RH to R.drawable.ic_wink_rh,
            Action.WINK_BOTH to R.drawable.ic_wink_both, Action.ALTERNATE to R.drawable.ic_swap,
            Action.STOP to R.drawable.ic_stop,
        )
        val current = Store.widgetAction(id)
        for (a in Action.entries) {
            val summary = when (a) {
                Action.ALTERNATE -> "${Store.altSeconds} s (set in Settings)"
                current -> "Current"
                else -> null
            }
            UiKit.row(card, icons.getValue(a), a.label, summary) {
                Store.setWidgetAction(id, a)
                AppWidgetManager.getInstance(this).updateAppWidget(id, ActionWidget.views(this, id))
                setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id))
                finish()
            }
        }
    }
}
