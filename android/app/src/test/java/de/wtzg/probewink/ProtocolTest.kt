package de.wtzg.probewink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Strings exactly as the firmware sends them (wink-app/src/master.rs, ble_phone.rs). */
class ProtocolTest {
    @Test fun states() {
        for (s in Protocol.STATES) assertEquals(Notice.State(s), parseNotice(s))
    }

    @Test fun trailingNulAndWhitespace() {
        assertEquals(Notice.State("idle"), parseNotice("idle\u0000\u0000"))
        assertEquals(Notice.State("idle"), parseNotice(" idle\n"))
    }

    @Test fun errors() {
        assertEquals(Notice.Error("busy"), parseNotice("error: busy"))
        assertEquals(Notice.Error("unknown command"), parseNotice("error: unknown command"))
        assertEquals(Notice.Error("resting", 12), parseNotice("error: resting 12"))
        assertEquals(Notice.Error("resting"), parseNotice("error: resting"))   // older firmware
    }

    @Test fun remoteAndVersion() {
        assertEquals(Notice.Remote("connected"), parseNotice("remote: connected"))
        assertEquals(Notice.Remote("none found"), parseNotice("remote: none found"))
        assertEquals(Notice.Version("0.1.0"), parseNotice("version: 0.1.0"))
    }

    @Test fun driveLock() {
        assertEquals(Notice.DriveLock(true), parseNotice("drivelock: on"))
        assertEquals(Notice.DriveLock(false), parseNotice("drivelock: off"))
        assertEquals("DRIVELOCK ON", Protocol.driveLock(true))
        assertEquals(
            "Refused: the engine is running and the drive lock is on.",
            describeError(Notice.Error("engine running")),
        )
    }

    @Test fun fingerprintMatchesFirmware() {
        // Same vector as wink-core proto::tests::fingerprint_depends_on_the_key_only.
        assertEquals("85f7eb12", Protocol.fingerprint(ByteArray(32) { it.toByte() }))
    }

    @Test fun couplingNotices() {
        assertEquals(Notice.Link(null), parseNotice("link: not coupled"))
        assertEquals(Notice.Link("1a2b3c4d", true), parseNotice("link: coupled 1a2b3c4d rh-ok"))
        assertEquals(Notice.Link("1a2b3c4d", false), parseNotice("link: coupled 1a2b3c4d rh-silent"))
        assertEquals(Notice.Link("1a2b3c4d", null), parseNotice("link: coupled 1a2b3c4d"))
        assertEquals(Notice.SideIs(Side.RH), parseNotice("side: rh"))
        assertEquals(Side.LH, Protocol.sideOf("PWink-L"))
        assertEquals(Side.RH, Protocol.sideOf("ProbeWink RH"))
        assertEquals(null, Protocol.sideOf("PWink"))
        assertTrue(Protocol.isModuleName("PWink-R"))
    }

    @Test fun unknown() {
        assertTrue(parseNotice("hello") is Notice.Unknown)
    }

    @Test fun commandsMatchTheFirmwareGrammar() {
        // command.rs: WINK L|R|B, ALT [n>0], STOP, STATUS, PAIR REMOTE, FORGET REMOTE; at most 32 bytes
        assertEquals(listOf("WINK L", "WINK R", "WINK B", "ALT", "STOP"), Action.entries.map { it.command })
        assertEquals("ALT 10", Protocol.alt(99))
        assertEquals("ALT 1", Protocol.alt(0))
        for (a in Action.entries) assertTrue(a.command.length <= Protocol.MAX_LEN)
    }

    @Test fun restingErrorText() {
        assertEquals(
            "Refused: alternate is locked for another 12 s. Winks still work.",
            describeError(Notice.Error("resting", 12)),
        )
    }
}
