//! ESP-NOW link between the modules.
//!
//! Wi-Fi runs in station mode on a fixed channel and never associates.
//! All frames are broadcast; authenticity and freshness come from
//! [`wink_core::proto`] (HMAC-SHA256 + epoch/sequence), not from ESP-NOW's
//! own encryption, which does not cover broadcast frames.

use std::sync::mpsc::SyncSender;

use esp_idf_svc::espnow::{EspNow, PeerInfo, BROADCAST};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{self, EspError};
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, WifiDriver};
use log::{info, warn};
use wink_core::config::ESPNOW_CHANNEL;
use wink_core::proto::{self, Counter, Message};

use crate::secrets::ESPNOW_KEY;
use crate::Event;

pub struct Link {
    _wifi: WifiDriver<'static>,
    now: EspNow<'static>,
    counter: Counter,
}

impl Link {
    /// Start Wi-Fi + ESP-NOW. Received frames go to `events` as
    /// [`Event::Radio`]; nothing is decoded in the Wi-Fi task.
    pub fn start(
        modem: Modem<'static>,
        sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
        counter: Counter,
        events: SyncSender<Event>,
    ) -> Result<Self, EspError> {
        let mut wifi = WifiDriver::new(modem, sysloop, Some(nvs))?;
        wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
        wifi.start()?;
        sys::esp!(unsafe {
            sys::esp_wifi_set_channel(ESPNOW_CHANNEL, sys::wifi_second_chan_t_WIFI_SECOND_CHAN_NONE)
        })?;

        // EspNow::take() also disables Wi-Fi modem sleep (needed for
        // reliable ESP-NOW reception).
        let now = EspNow::take()?;
        let mut peer: PeerInfo = unsafe { core::mem::zeroed() };
        peer.peer_addr = BROADCAST;
        peer.channel = ESPNOW_CHANNEL;
        peer.ifidx = sys::wifi_interface_t_WIFI_IF_STA;
        peer.encrypt = false;
        now.add_peer(peer)?;
        now.register_recv_cb(move |_info, data| {
            if data.len() == proto::FRAME_LEN {
                // Never block the Wi-Fi task; drop frames if the queue is full.
                let _ = events.try_send(Event::Radio(data.to_vec()));
            }
        })?;
        info!("ESP-NOW up on channel {ESPNOW_CHANNEL}, epoch {}", counter.epoch());
        Ok(Self { _wifi: wifi, now, counter })
    }

    pub fn send(&mut self, msg: Message) {
        let frame = proto::encode(&ESPNOW_KEY, self.counter.next(), msg);
        if let Err(e) = self.now.send(BROADCAST, &frame) {
            warn!("ESP-NOW send {msg:?} failed: {e}");
        }
    }

    pub fn counter_mut(&mut self) -> &mut Counter {
        &mut self.counter
    }
}
