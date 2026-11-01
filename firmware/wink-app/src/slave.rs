//! RH module: follows the master's relay frames, with its own safety rules.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use esp_idf_svc::hal::task::watchdog::WatchdogSubscription;
use log::{info, warn};
use wink_core::awake::AwakeTimer;
use wink_core::config::{SUPPLY, TIMING};
use wink_core::proto::{self, Freshness, Message, ReplayGuard, Role, SupplyState};
use wink_core::safety::{HeartbeatWatchdog, OnTimeLimiter};
use wink_core::voltage::{Fault, SupplyGuard};
use wink_core::Millis;

use crate::hw::Board;
use crate::link::Link;
use crate::secrets::ESPNOW_KEY;
use crate::store::Store;
use crate::{millis, sleep, Event};

const TICK: Duration = Duration::from_millis(20);
/// At most one StaleEpoch reply per this interval.
const STALE_REPLY_MS: Millis = 1000;

pub struct Slave {
    pub board: Board,
    pub link: Link,
    pub store: Store,
    pub events: Receiver<Event>,
}

impl Slave {
    pub fn run(mut self, wdt: &mut WatchdogSubscription<'_>) -> ! {
        let mut rx = ReplayGuard::from_persisted(self.store.rx_epoch());
        let mut hb = HeartbeatWatchdog::new(TIMING.heartbeat_timeout_ms);
        let mut limit = OnTimeLimiter::new(TIMING.relay_max_on_ms);
        let mut supply = SupplyGuard::new(SUPPLY);
        let mut awake = AwakeTimer::new(millis(), TIMING.awake_window_ms);
        let mut last_reported: Option<(bool, bool, SupplyState)> = None;
        let mut last_stale_reply: Option<Millis> = None;
        let mut sleep_requested = false;
        info!("slave: last accepted master epoch {}", rx.epoch());

        loop {
            let _ = wdt.feed();

            let mut frames = Vec::new();
            match self.events.recv_timeout(TICK) {
                Ok(ev) => {
                    frames.push(ev);
                    while let Ok(ev) = self.events.try_recv() {
                        frames.push(ev);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => warn!("event channel closed"),
            }
            let now = millis();
            let mut status_requested = false;

            for ev in frames {
                let Event::Radio(frame) = ev else { continue };
                let Ok((h, msg)) = proto::decode(&ESPNOW_KEY, &frame) else {
                    continue; // not ours, or forged
                };
                if h.sender != Role::Master {
                    continue;
                }
                match rx.check(h.epoch, h.seq) {
                    Freshness::Stale => {
                        // Authentic but old. If the master is behind (lost its
                        // counter), tell it where we are.
                        let due = last_stale_reply.is_none_or(|t| now.saturating_sub(t) >= STALE_REPLY_MS);
                        if rx.is_behind(h.epoch) && due {
                            last_stale_reply = Some(now);
                            self.link.send(Message::StaleEpoch { last_epoch: rx.epoch() });
                        }
                        continue;
                    }
                    Freshness::NewEpoch => {
                        info!("slave: master epoch {}", h.epoch);
                        self.store.set_rx_epoch(h.epoch);
                    }
                    Freshness::Fresh => {}
                }
                awake.activity(now);
                match msg {
                    Message::Relay { on } => hb.on_frame(now, on),
                    Message::Ping => status_requested = true,
                    Message::Sleep => sleep_requested = true,
                    _ => {}
                }
            }

            // Relay: heartbeat → supply gate → on-time limit → pin.
            let want = hb.relay_wanted(now);
            let energizing = want && !self.board.relay_on();
            if supply.reading_due(now, energizing) {
                match self.board.read_supply() {
                    Ok(r) => {
                        supply.set_temp_c(self.board.die_temp_c());
                        supply.update(now, r, self.board.relay_on())
                    }
                    Err(e) => warn!("supply ADC read failed: {e}"),
                }
            }
            let on = want && supply.permits(now, self.board.relay_on());
            let on = limit.apply(now, on);
            self.board.set_relay(on);
            self.board.set_led(on);

            let sup = match supply.fault() {
                None => SupplyState::Ok,
                Some(Fault::Overvoltage) => SupplyState::Over,
                Some(Fault::Undervoltage) => SupplyState::Under,
            };
            let report = (on, limit.tripped(), sup);
            if status_requested || last_reported != Some(report) {
                last_reported = Some(report);
                self.link.send(Message::Status { relay_on: on, limit_tripped: report.1, supply: sup });
            }

            if sleep_requested || awake.should_sleep(now, on) {
                info!("slave: sleeping ({})", if sleep_requested { "master asked" } else { "timeout" });
                hb.clear();
                sleep::enter(&mut self.board);
            }
        }
    }
}
