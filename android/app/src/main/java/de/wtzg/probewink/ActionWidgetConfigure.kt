package de.wtzg.probewink

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

/** Shown when a single-action widget is placed (or reconfigured): pick its action. */
class ActionWidgetConfigure : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.init(this)
        val id = intent?.extras?.getInt(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID)
            ?: AppWidgetManager.INVALID_APPWIDGET_ID
        setResult(RESULT_CANCELED, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id))
        if (id == AppWidgetManager.INVALID_APPWIDGET_ID) { finish(); return }

        setContentView(R.layout.activity_configure)
        val list = findViewById<LinearLayout>(R.id.cfg_list)
        findViewById<TextView>(R.id.cfg_title).text = getString(R.string.cfg_title)
        for (a in Action.entries) {
            val b = layoutInflater.inflate(R.layout.item_action, list, false) as Button
            b.text = if (a == Action.ALTERNATE) "${a.label} (${Store.altSeconds} s)" else a.label
            b.setOnClickListener {
                Store.setWidgetAction(id, a)
                AppWidgetManager.getInstance(this).updateAppWidget(id, ActionWidget.views(this, id))
                setResult(RESULT_OK, Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, id))
                finish()
            }
            list.addView(b)
        }
    }
}
