package de.wtzg.probewink

import android.app.Activity
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.LinearLayout

/**
 * Coupling wizard: pick both modules, then generate a random link key, write it to
 * both and check through the LH module that the two now talk to each other.
 */
class CouplingActivity : Activity(), Coupler.Ui {

    private lateinit var list: LinearLayout
    private lateinit var lhRow: UiKit.Row
    private lateinit var rhRow: UiKit.Row
    private lateinit var steps: Map<Coupler.Step, UiKit.Row>
    private lateinit var button: Button
    private var coupler: Coupler? = null
    private var running = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.init(this)
        setContentView(R.layout.activity_list)
        UiKit.topBar(this, "Couple modules", back = true)
        list = findViewById(R.id.list)

        UiKit.note(list,
            "The two modules talk over an authenticated radio link. Coupling gives both the same new random key. " +
                "Wake both modules first (switch the headlamps once), and keep the phone near the car.", top = 4)

        val mods = UiKit.section(list, "Modules")
        lhRow = UiKit.row(mods, R.drawable.ic_bluetooth, "LH module", null, chevron = true) { choose(Side.LH) }
        rhRow = UiKit.row(mods, R.drawable.ic_bluetooth, "RH module", null, chevron = true) { choose(Side.RH) }

        val prog = UiKit.section(list, "Steps")
        steps = mapOf(
            Coupler.Step.KEY to UiKit.row(prog, R.drawable.ic_lock, "Generate a new key"),
            Coupler.Step.WRITE_LH to UiKit.row(prog, R.drawable.ic_link, "Write it to the LH module"),
            Coupler.Step.WRITE_RH to UiKit.row(prog, R.drawable.ic_link, "Write it to the RH module"),
            Coupler.Step.VERIFY to UiKit.row(prog, R.drawable.ic_check, "Check that both talk"),
        )

        button = Button(this, null, 0, R.style.Btn_Primary).apply {
            text = "Couple"
            setOnClickListener { if (running) cancel() else start() }
        }
        list.addView(button, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            topMargin = UiKit.dp(this@CouplingActivity, 24)
        })
        renderModules()
    }

    private fun renderModules() {
        lhRow.summary(Store.address?.let { "${Store.deviceName ?: "Module"} · $it" } ?: "Choose the LH module")
        rhRow.summary(Store.rhAddress?.let { "${Store.rhName ?: "Module"} · $it" } ?: "Choose the RH module")
        button.isEnabled = Store.address != null || running
    }

    @android.annotation.SuppressLint("MissingPermission")   // hasBluetooth() checked first
    private fun choose(side: Side) {
        if (running) return
        if (!MainActivity.hasBluetooth(this)) {
            requestPermissions(MainActivity.neededPermissions().toTypedArray(), 1)
            return
        }
        ModulePicker.pick(this, side, onPicked = { d ->
            if (side == Side.LH) { Store.address = d.address; Store.deviceName = d.name ?: "ProbeWink LH" }
            else { Store.rhAddress = d.address; Store.rhName = d.name ?: "ProbeWink RH" }
            renderModules()
        })
    }

    private fun start() {
        val lh = Store.address ?: return
        val rh = Store.rhAddress
        if (rh == null) {
            android.app.AlertDialog.Builder(this)
                .setTitle("No RH module chosen")
                .setMessage("Only the LH module gets the new key. The RH lamp won't follow until you couple both. Useful for a bench test with one module.")
                .setPositiveButton("Couple LH only") { _, _ -> run(lh, null) }
                .setNegativeButton("Cancel", null)
                .show()
            return
        }
        run(lh, rh)
    }

    private fun run(lh: String, rh: String?) {
        running = true
        button.text = "Cancel"
        steps.values.forEach { it.summary(null); UiKit.status(it, null) }
        // The app's own connection would compete for the module during the restart.
        WinkService.disconnect(this)
        coupler = Coupler(this, this).also { it.start(lh, rh) }
    }

    private fun cancel() {
        coupler?.cancel()
        coupler = null
        running = false
        button.text = "Couple"
        renderModules()
    }

    override fun onStep(step: Coupler.Step, result: Coupler.Result, detail: String?) {
        val row = steps.getValue(step)
        row.summary(detail)
        UiKit.status(row, result)
    }

    override fun onFinished(ok: Boolean, fingerprint: String?) {
        running = false
        coupler = null
        button.text = if (ok) "Done" else "Try again"
        button.setOnClickListener { if (ok) finish() else if (running) cancel() else start() }
        if (ok) WinkService.connect(this)
        renderModules()
    }

    override fun onDestroy() {
        coupler?.cancel()
        super.onDestroy()
    }

    @Suppress("unused")
    private fun View.gone() { visibility = View.GONE }
}
