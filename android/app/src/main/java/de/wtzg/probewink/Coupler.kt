package de.wtzg.probewink

import android.content.Context
import android.os.Handler
import android.os.Looper
import java.security.SecureRandom

/**
 * Couples the two modules: generate a random 256-bit link key, write it to the LH
 * and the RH module over their bonded, encrypted links, then verify through the LH
 * module that it holds the expected key (fingerprint) and hears the RH module.
 * Each module restarts after storing the key. Runs on the main thread.
 */
class Coupler(private val context: Context, private val ui: Ui) {

    enum class Step { KEY, WRITE_LH, WRITE_RH, VERIFY }
    enum class Result { RUNNING, OK, FAILED, SKIPPED }

    interface Ui {
        fun onStep(step: Step, result: Result, detail: String?)
        fun onFinished(ok: Boolean, fingerprint: String?)
    }

    private val main = Handler(Looper.getMainLooper())
    private var link: WinkLink? = null
    private var cancelled = false

    fun start(lh: String, rh: String?) {
        cancelled = false
        val key = ByteArray(Protocol.KEY_LEN).also { SecureRandom().nextBytes(it) }
        val fp = Protocol.fingerprint(key)
        ui.onStep(Step.KEY, Result.OK, "Fingerprint $fp")
        Store.log("coupling: new key, fingerprint $fp")
        write(lh, key, Step.WRITE_LH) { okLh ->
            if (!okLh) return@write finish(false, null)
            if (rh == null) {
                ui.onStep(Step.WRITE_RH, Result.SKIPPED, "No RH module chosen")
                return@write verify(lh, fp, expectRh = false)
            }
            write(rh, key, Step.WRITE_RH) { okRh ->
                if (!okRh) return@write finish(false, fp)
                verify(lh, fp, expectRh = true)
            }
        }
    }

    fun cancel() {
        cancelled = true
        link?.close()
        link = null
    }

    /** Connect, write the key, wait for the module to restart. */
    private fun write(address: String, key: ByteArray, step: Step, then: (Boolean) -> Unit) {
        ui.onStep(step, Result.RUNNING, "Connecting…")
        var settled = false
        val l = WinkLink(context, object : WinkLink.Events {
            override fun onLink(link: Link, problem: String?) {
                if (link == Link.CONNECTED) ui.onStep(step, Result.RUNNING, "Writing the key…")
                if (link == Link.DISCONNECTED && problem != null && !settled) {
                    settled = true
                    ui.onStep(step, Result.FAILED, problem)
                    then(false)
                }
            }
            override fun onNotice(notice: Notice) = Unit
        })
        link = l
        l.connect(address)
        l.writeKey(key) { ok ->
            if (settled || cancelled) return@writeKey
            settled = true
            if (!ok) {
                l.close()
                ui.onStep(step, Result.FAILED, "The module did not accept the key. Is its firmware current?")
                then(false)
                return@writeKey
            }
            ui.onStep(step, Result.OK, "Key stored; module restarting")
            // The module restarts by itself; drop our side and give it time to boot.
            main.postDelayed({ l.close(); if (!cancelled) then(true) }, 1500)
        }
    }

    /** Ask the LH module for STATUS: fingerprint must match, RH should be heard. */
    private fun verify(lh: String, fp: String, expectRh: Boolean) {
        ui.onStep(Step.VERIFY, Result.RUNNING, "Waiting for the modules to restart…")
        var attempt = 0
        fun tryOnce() {
            if (cancelled) return
            attempt++
            var answered = false
            val l = WinkLink(context, object : WinkLink.Events {
                override fun onLink(link: Link, problem: String?) {
                    if (link == Link.DISCONNECTED && problem != null && !answered) {
                        answered = true
                        retryOrFail("LH module not reachable after the restart.")
                    }
                }
                override fun onNotice(notice: Notice) {
                    if (notice !is Notice.Link || answered) return
                    when {
                        notice.fingerprint != fp -> {
                            answered = true
                            this@Coupler.link?.close()
                            ui.onStep(Step.VERIFY, Result.FAILED, "LH reports key ${notice.fingerprint ?: "none"}, expected $fp.")
                            finish(false, fp)
                        }
                        expectRh && notice.rhSeen != true -> {
                            answered = true
                            this@Coupler.link?.close()
                            retryOrFail("LH has the key but does not hear the RH module yet.")
                        }
                        else -> {
                            answered = true
                            this@Coupler.link?.close()
                            ui.onStep(
                                Step.VERIFY, Result.OK,
                                if (expectRh) "Both modules hold key $fp and talk to each other" else "LH holds key $fp",
                            )
                            finish(true, fp)
                        }
                    }
                }

                fun retryOrFail(why: String) {
                    if (attempt < 4) {
                        ui.onStep(Step.VERIFY, Result.RUNNING, "$why Retrying ($attempt/3)…")
                        main.postDelayed({ tryOnce() }, 2500)
                    } else {
                        ui.onStep(Step.VERIFY, Result.FAILED, why)
                        finish(false, fp)
                    }
                }
            })
            link = l
            l.connect(lh)
        }
        main.postDelayed({ tryOnce() }, 3000)
    }

    private fun finish(ok: Boolean, fp: String?) {
        if (ok) Store.setCoupled(fp)
        Store.log("coupling ${if (ok) "done" else "failed"}")
        ui.onFinished(ok, fp)
    }
}
