package de.wtzg.probewink

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.view.View
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast

/** Connection log: every command, reply and link event since the app started. */
class LogActivity : Activity() {
    private val render: () -> Unit = { render() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.init(this)
        setContentView(R.layout.activity_log)
        UiKit.topBar(
            this, "Connection log", back = true,
            action = R.drawable.ic_delete to { Store.clearLog() },
            action2 = R.drawable.ic_copy to {
                getSystemService(ClipboardManager::class.java)
                    .setPrimaryClip(ClipData.newPlainText("ProbeWink log", Store.logText()))
                Toast.makeText(this, "Log copied", Toast.LENGTH_SHORT).show()
            },
            actionDesc = "Clear", action2Desc = "Copy",
        )
    }

    override fun onStart() {
        super.onStart()
        Store.listen(render)
        render()
    }

    override fun onStop() {
        Store.unlisten(render)
        super.onStop()
    }

    private fun render() {
        val text = Store.logText()
        findViewById<TextView>(R.id.log).text = text.ifEmpty { "Nothing yet. Connect to the module to see traffic here." }
        val scroll = findViewById<ScrollView>(R.id.log_scroll)
        scroll.post { scroll.fullScroll(View.FOCUS_DOWN) }
    }
}
