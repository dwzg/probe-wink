//! LH module: phone + remote over BLE, LH relay, RH relay over ESP-NOW.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use esp_idf_svc::hal::task::watchdog::WatchdogSubscription;
use log::{info, warn};
use wink_core::awake::AwakeTimer;
use wink_core::command::{self, Command};
use wink_core::config::{REMOTE_TIMING, SUPPLY, TIMING};
use wink_core::controller::{Controller, Reject, Status};
use wink_core::proto::{self, Message, ReplayGuard, Role, SupplyState};
use wink_core::remote::{pattern_command, Edge, PressDecoder};
use wink_core::safety::{HeartbeatSender, OnTimeLimiter};
use wink_core::voltage::{Fault, SupplyGuard, engine_running};
use wink_core::Millis;

use crate::ble_phone::Phone;
use crate::ble_remote::RemoteCtl;
use crate::hw::Board;
use crate::link::Link;
use crate::coupling;
use crate::hw::Side;
use crate::store::Store;
use crate::{millis, sleep, Event};

/// Forget a slave supply fault if no status arrived for this long.
const SLAVE_STATUS_TTL_MS: Millis = 12_000;
/// Loop period.
const TICK: Duration = Duration::from_millis(20);

pub struct Master {
    pub board: Board,
    pub link: Link,
    pub phone: Phone,
    pub remote: Arc<RemoteCtl>,
    pub store: Store,
    pub events: Receiver<Event>,
}

struct State {
    ctl: Controller,
    lh_limit: OnTimeLimiter,
    supply: SupplyGuard,
    hb: HeartbeatSender,
    decoder: PressDecoder,
    awake: AwakeTimer,
    slave_rx: ReplayGuard,
    slave_fault: Option<(Fault, Millis)>,
    last_ping: Millis,
    last_status: Option<Status>,
    /// Last remote link report, resent on STATUS.
    remote_up: bool,
    /// Refuse winks / ALT while the engine runs (NVS).
    drive_lock: bool,
    /// Last authenticated status frame from the RH module.
    slave_seen: Option<Millis>,
}

impl Master {
    pub fn run(mut self, wdt: &mut WatchdogSubscription<'_>) -> ! {
        let now = millis();
        let mut st = State {
            ctl: Controller::new(TIMING),
            lh_limit: OnTimeLimiter::new(TIMING.relay_max_on_ms),
            supply: SupplyGuard::new(SUPPLY),
            hb: HeartbeatSender::new(&TIMING),
            decoder: PressDecoder::new(REMOTE_TIMING),
            awake: AwakeTimer::new(now, TIMING.awake_window_ms),
            // Slave frames: in-memory replay protection only (see README).
            slave_rx: ReplayGuard::from_persisted(0),
            slave_fault: None,
            last_ping: now,
            last_status: None,
            remote_up: false,
            drive_lock: self.store.drive_lock(),
            slave_seen: None,
        };
        self.link.send(Message::Ping);

        loop {
            let _ = wdt.feed();

            match self.events.recv_timeout(TICK) {
                Ok(ev) => {
                    self.handle(&mut st, ev);
                    while let Ok(ev) = self.events.try_recv() {
                        self.handle(&mut st, ev);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => warn!("event channel closed"),
            }
            let now = millis();

            // Remote patterns that complete by timeout.
            if let Some(p) = st.decoder.poll(now) {
                let cmd = pattern_command(p, st.ctl.is_alternating(), TIMING.alt_default_s);
                info!("remote: {p:?} -> {cmd:?}");
                self.command(&mut st, now, cmd);
            }

            // Supply voltage: read, then feed faults into the controller.
            let want = st.ctl.tick(now);
            let energizing = want.lh && !self.board.relay_on();
            if st.supply.reading_due(now, energizing) {
                match self.board.read_supply() {
                    Ok(r) => {
                        st.supply.set_temp_c(self.board.die_temp_c());
                        st.supply.update(now, r, self.board.relay_on())
                    }
                    Err(e) => warn!("supply ADC read failed: {e}"),
                }
            }
            if st.slave_fault.is_some_and(|(_, at)| now.saturating_sub(at) > SLAVE_STATUS_TTL_MS) {
                st.slave_fault = None;
            }
            let fault = st.supply.fault().or(st.slave_fault.map(|f| f.0));
            st.ctl.set_inhibit(now, fault);

            // Relays: controller → supply gate → on-time limit → pin.
            let want = st.ctl.tick(now);
            let lh = want.lh && st.supply.permits(now, self.board.relay_on());
            let lh = st.lh_limit.apply(now, lh);
            if want.lh && !lh && st.lh_limit.tripped() {
                warn!("LH relay on-time limit reached");
            }
            self.board.set_relay(lh);
            let blink = self.remote.pairing() && (now / 250) % 2 == 0;
            self.board.set_led(lh || want.rh || blink);

            // RH relay frames (heartbeat while on) and keep-alive pings.
            if let Some(on) = st.hb.poll(now, want.rh) {
                self.link.send(Message::Relay { on });
            }
            if now.saturating_sub(st.last_ping) >= TIMING.ping_interval_ms as Millis {
                st.last_ping = now;
                self.link.send(Message::Ping);
            }

            // State notifications on change.
            let status = st.ctl.status();
            if st.last_status != Some(status) {
                st.last_status = Some(status);
                info!("state: {}", status.as_str());
                self.phone.set_state(status.as_str());
            }

            crate::ota::start_if_requested();

            let busy = st.ctl.is_running() || lh || want.rh || self.remote.pairing();
            if st.awake.should_sleep(now, busy) {
                self.go_to_sleep(&mut st);
            }
        }
    }

    fn handle(&mut self, st: &mut State, ev: Event) {
        let now = millis();
        match ev {
            Event::PhoneWrite(data) => match command::parse(&data, TIMING.alt_default_s) {
                Ok(cmd) => {
                    info!("phone: {cmd:?}");
                    self.command(st, now, cmd);
                }
                Err(e) => {
                    warn!("phone: bad command ({})", e.as_str());
                    self.phone.reply(&format!("error: {}", e.as_str()));
                }
            },
            Event::RemoteEdge(edge, t) => {
                let p = match edge {
                    Edge::Press => st.decoder.press(t),
                    Edge::Release => st.decoder.release(t),
                };
                if let Some(p) = p {
                    let cmd = pattern_command(p, st.ctl.is_alternating(), TIMING.alt_default_s);
                    info!("remote: {p:?} -> {cmd:?}");
                    self.command(st, now, cmd);
                }
            }
            Event::RemoteLink(up) => {
                if !up {
                    st.decoder.reset();
                }
                st.remote_up = up;
                self.phone.reply(if up { "remote: connected" } else { "remote: disconnected" });
            }
            Event::RemoteInfo(text) => {
                info!("{text}");
                self.phone.reply(text);
            }
            Event::Radio(frame) => self.radio(st, now, &frame),
            Event::LinkKey(k) => coupling::apply_key(&mut self.store, &self.phone, k),
        }
    }

    fn command(&mut self, st: &mut State, now: Millis, cmd: Command) {
        st.awake.activity(now);
        match cmd {
            Command::Status => {
                // State, then everything the phone cannot read otherwise.
                self.phone.set_state(st.ctl.status().as_str());
                self.phone.reply(if st.remote_up { "remote: connected" } else { "remote: disconnected" });
                self.phone.reply(concat!("version: ", env!("CARGO_PKG_VERSION")));
                self.phone.reply(if st.drive_lock { "drivelock: on" } else { "drivelock: off" });
                self.phone.reply(coupling::side_line(Side::Lh));
                let rh = match st.slave_seen {
                    Some(t) if now.saturating_sub(t) <= SLAVE_STATUS_TTL_MS => " rh-ok",
                    _ => " rh-silent",
                };
                let link = coupling::link_line(self.link.key());
                if self.link.key().is_some() {
                    self.phone.reply(&(link + rh));
                } else {
                    self.phone.reply(&link);
                }
            }
            Command::Uncouple => coupling::uncouple(&mut self.store, &self.phone),
            c if c.needs_rh() && self.link.key().is_none() => {
                info!("{c:?} refused: not coupled");
                self.phone.reply(&format!("error: {}", Reject::NotCoupled.as_str()));
            }
            Command::DriveLock(on) => {
                st.drive_lock = on;
                self.store.set_drive_lock(on);
                info!("drive lock {}", if on { "on" } else { "off" });
                self.phone.reply(if on { "drivelock: on" } else { "drivelock: off" });
            }
            // Same path for phone and remote: refused before the controller sees it.
            c if st.drive_lock && c.moves_lamps() && engine_running(&SUPPLY, st.supply.last_rail_mv()) => {
                info!("{c:?} refused: engine running (drive lock)");
                self.phone.reply(&format!("error: {}", Reject::EngineRunning.as_str()));
            }
            Command::PairRemote => self.remote.request_pair(),
            Command::ForgetRemote => self.remote.request_forget(),
            _ => {
                if let Err(r) = st.ctl.command(now, cmd) {
                    info!("{cmd:?} refused: {}", r.as_str());
                    match r {
                        // Seconds left, rounded up, so the phone can count down.
                        Reject::Resting { remaining_ms } => {
                            self.phone.reply(&format!("error: resting {}", remaining_ms.div_ceil(1000)))
                        }
                        _ => self.phone.reply(&format!("error: {}", r.as_str())),
                    }
                }
            }
        }
    }

    fn radio(&mut self, st: &mut State, now: Millis, frame: &[u8]) {
        let Some(key) = self.link.key().copied() else { return };
        let Ok((h, msg)) = proto::decode(&key, frame) else {
            return; // not ours, or forged
        };
        if h.sender != Role::Slave {
            return;
        }
        if st.slave_rx.check(h.epoch, h.seq) == proto::Freshness::Stale {
            return;
        }
        match msg {
            Message::StaleEpoch { last_epoch } => {
                if let Some(e) = self.link.counter_mut().bump_past(last_epoch) {
                    warn!("slave saw epoch {last_epoch}; jumping to {e}");
                    self.store.set_tx_epoch(e);
                }
            }
            Message::Status { relay_on, limit_tripped, supply } => {
                st.slave_seen = Some(now);
                info!("slave: relay={relay_on} limit={limit_tripped} supply={supply:?}");
                st.slave_fault = match supply {
                    SupplyState::Ok => None,
                    SupplyState::Over => Some((Fault::Overvoltage, now)),
                    SupplyState::Under => Some((Fault::Undervoltage, now)),
                };
            }
            _ => {}
        }
    }

    fn go_to_sleep(&mut self, st: &mut State) -> ! {
        info!("awake window over, going to sleep");
        st.ctl.command(millis(), Command::Stop).ok();
        self.board.set_relay(false);
        self.phone.set_state(Status::Asleep.as_str());
        for _ in 0..3 {
            self.link.send(Message::Relay { on: false });
            self.link.send(Message::Sleep);
            std::thread::sleep(Duration::from_millis(30));
        }
        sleep::enter(&mut self.board)
    }
}
