//! The PC side of the Bluetooth link: advertises one GATT service the phone page connects to.
//! Nothing goes over Wi-Fi and there is no server.
use super::log;
use super::settings::Settings;
use airwand::protocol::{self, Event, LinkState, Packet};
use airwand::mux::Mux;
use airwand::validate;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{Result, GUID};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristicProperties, GattLocalCharacteristic, GattLocalCharacteristicParameters, GattProtectionLevel,
    GattReadRequestedEventArgs, GattServiceProvider, GattServiceProviderAdvertisementStatus,
    GattServiceProviderAdvertisementStatusChangedEventArgs, GattServiceProviderAdvertisingParameters,
    GattSubscribedClient, GattWriteOption, GattWriteRequestedEventArgs,
};
use windows::Devices::Bluetooth::{BluetoothAdapter, BluetoothError};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};
use windows::core::IInspectable;
use airwand::server::Sink;

/// WinRT GATT objects are agile (free-threaded); this lets us keep them in shared state.
struct Agile<T>(T);
unsafe impl<T> Send for Agile<T> {}
unsafe impl<T> Sync for Agile<T> {}

struct Peer {
    device_id: Option<String>,
    name: String,
    model: String,
    battery: Option<u8>,
    state: LinkState,
    paused: bool,
    subscribed: bool,
    last_seen: Instant,
}

impl Peer {
    fn new() -> Self {
        Self {
            device_id: None,
            name: "Unknown device".into(),
            model: String::new(),
            battery: None,
            state: LinkState::Waiting,
            paused: false,
            subscribed: false,
            last_seen: Instant::now(),
        }
    }
}

/// What the control panel shows for one connected device.
pub struct PeerInfo {
    pub name: String,
    pub model: String,
    pub battery: Option<u8>,
    pub state: LinkState,
    pub paused: bool,
    pub wifi: bool,
    pub ble: bool,
    pub seen_ms: u64,
    pub code: String,
}

#[derive(Default)]
struct Inner {
    peers: HashMap<String, Peer>,
    asking: HashSet<String>,
    denied_at: HashMap<String, Instant>,
    prompts: VecDeque<Instant>,
}


/// (key, peer paused, event) for every approved event.
pub type EventFn = Box<dyn Fn(&str, bool, Event) + Send + Sync>;
/// (device name, pairing code) -> allowed?
pub type AskFn = Box<dyn Fn(&str, &str) -> bool + Send + Sync>;

pub struct Receiver {
    inner: Mutex<Inner>,
    /// Device-level view over all links of a device (dedupe, pause, stuck-button release).
    mux: Mutex<Mux>,
    settings: Arc<Mutex<Settings>>,
    on_event: EventFn,
    ask: AskFn,
    status: Mutex<Option<Agile<GattLocalCharacteristic>>>,
    wifi: Mutex<HashMap<String, Sink>>,
    provider: Mutex<Option<Agile<GattServiceProvider>>>,
    pub error: Mutex<Option<String>>,
    pub advertising: AtomicBool,
    pub paused: AtomicBool,
    packets: AtomicU64,
    aims: AtomicU64,
    rejected: AtomicU64,
    pub pc_name: String,
}

fn make_buffer(bytes: &[u8]) -> Result<IBuffer> {
    let w = DataWriter::new()?;
    w.WriteBytes(bytes)?;
    w.DetachBuffer()
}

fn read_buffer(buf: &IBuffer) -> Result<Vec<u8>> {
    let r = DataReader::FromBuffer(buf)?;
    let mut v = vec![0u8; r.UnconsumedBufferLength()? as usize];
    r.ReadBytes(&mut v)?;
    Ok(v)
}

fn client_key(c: &GattSubscribedClient) -> Option<String> {
    Some(c.Session().ok()?.DeviceId().ok()?.Id().ok()?.to_string())
}

impl Receiver {
    pub fn new(settings: Arc<Mutex<Settings>>, on_event: EventFn, ask: AskFn) -> Arc<Self> {
        let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            mux: Mutex::new(Mux::new()),
            settings,
            on_event,
            ask,
            status: Mutex::new(None),
            wifi: Mutex::new(HashMap::new()),
            provider: Mutex::new(None),
            error: Mutex::new(None),
            advertising: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            packets: AtomicU64::new(0),
            aims: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            pc_name: name.chars().take(24).collect(),
        })
    }

    pub fn aims(&self) -> u64 {
        self.aims.load(Ordering::Relaxed)
    }
    pub fn rejected(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    fn fail(&self, msg: String) {
        log::write(&msg);
        *self.error.lock().unwrap() = Some(msg);
    }

    /// Blocks while setting up; run it on its own thread.
    pub fn start(self: &Arc<Self>) {
        if let Err(e) = self.start_inner() {
            self.fail(format!("Bluetooth error: {e}"));
        }
    }

    fn start_inner(self: &Arc<Self>) -> Result<()> {
        let adapter = match BluetoothAdapter::GetDefaultAsync().and_then(|op| op.get()) {
            Ok(a) => a,
            Err(_) => {
                self.fail("No Bluetooth adapter found.".into());
                return Ok(());
            }
        };
        if !adapter.IsPeripheralRoleSupported()? {
            self.fail(
                "This Bluetooth adapter cannot act as a peripheral (BLE peripheral role). Try a newer adapter or a USB Bluetooth 4.0+ dongle."
                    .into(),
            );
            return Ok(());
        }
        let res = GattServiceProvider::CreateAsync(GUID::from_u128(protocol::SERVICE))?.get()?;
        if res.Error()? != BluetoothError::Success {
            self.fail(format!("Could not create service: {:?}", res.Error()?));
            return Ok(());
        }
        let provider = res.ServiceProvider()?;
        let level = if self.settings.lock().unwrap().require_encryption {
            GattProtectionLevel::EncryptionRequired
        } else {
            GattProtectionLevel::Plain
        };

        let in_params = GattLocalCharacteristicParameters::new()?;
        in_params.SetCharacteristicProperties(GattCharacteristicProperties::WriteWithoutResponse)?;
        in_params.SetWriteProtectionLevel(level)?;
        let in_res = provider.Service()?.CreateCharacteristicAsync(GUID::from_u128(protocol::INPUT), &in_params)?.get()?;

        let st_params = GattLocalCharacteristicParameters::new()?;
        st_params.SetCharacteristicProperties(GattCharacteristicProperties::Read | GattCharacteristicProperties::Notify)?;
        st_params.SetReadProtectionLevel(level)?;
        let st_res = provider.Service()?.CreateCharacteristicAsync(GUID::from_u128(protocol::STATUS), &st_params)?.get()?;

        if in_res.Error()? != BluetoothError::Success || st_res.Error()? != BluetoothError::Success {
            self.fail("Could not create characteristics.".into());
            return Ok(());
        }
        let input = in_res.Characteristic()?;
        let status = st_res.Characteristic()?;
        *self.status.lock().unwrap() = Some(Agile(status.clone()));

        let me = self.clone();
        let on_write: TypedEventHandler<GattLocalCharacteristic, GattWriteRequestedEventArgs> =
            TypedEventHandler::new(move |_: &Option<GattLocalCharacteristic>, args: &Option<GattWriteRequestedEventArgs>| {
                if let Some(a) = args.as_ref() {
                    if let Err(e) = me.on_write(a) {
                        log::write(&format!("write handler: {e}"));
                    }
                }
                Ok(())
            });
        input.WriteRequested(&on_write)?;

        let me = self.clone();
        let on_read: TypedEventHandler<GattLocalCharacteristic, GattReadRequestedEventArgs> =
            TypedEventHandler::new(move |_: &Option<GattLocalCharacteristic>, args: &Option<GattReadRequestedEventArgs>| {
                if let Some(a) = args.as_ref() {
                    let _ = me.on_read(a);
                }
                Ok(())
            });
        status.ReadRequested(&on_read)?;

        let me = self.clone();
        let on_subs: TypedEventHandler<GattLocalCharacteristic, IInspectable> = TypedEventHandler::new(move |sender: &Option<GattLocalCharacteristic>, _: &Option<IInspectable>| {
            if let Some(ch) = sender.as_ref() {
                me.on_subscribers(ch);
            }
            Ok(())
        });
        status.SubscribedClientsChanged(&on_subs)?;

        let me = self.clone();
        let on_adv: TypedEventHandler<GattServiceProvider, GattServiceProviderAdvertisementStatusChangedEventArgs> =
            TypedEventHandler::new(move |sender: &Option<GattServiceProvider>, _: &Option<GattServiceProviderAdvertisementStatusChangedEventArgs>| {
                if let Some(p) = sender.as_ref() {
                    let on = p.AdvertisementStatus().ok() == Some(GattServiceProviderAdvertisementStatus::Started);
                    me.advertising.store(on, Ordering::Relaxed);
                }
                Ok(())
            });
        provider.AdvertisementStatusChanged(&on_adv)?;

        let adv = GattServiceProviderAdvertisingParameters::new()?;
        adv.SetIsDiscoverable(true)?;
        adv.SetIsConnectable(true)?;
        provider.StartAdvertisingWithParameters(&adv)?;
        *self.provider.lock().unwrap() = Some(Agile(provider));
        log::write("advertising started");
        Ok(())
    }

    // GATT callbacks

    fn on_write(self: &Arc<Self>, args: &GattWriteRequestedEventArgs) -> Result<()> {
        let deferral = args.GetDeferral()?;
        let r = (|| -> Result<()> {
            let req = args.GetRequestAsync()?.get()?;
            let data = read_buffer(&req.Value()?)?;
            if req.Option()? == GattWriteOption::WriteWithResponse {
                req.Respond()?;
            }
            let key = args.Session()?.DeviceId()?.Id()?.to_string();
            self.handle(&key, &data);
            Ok(())
        })();
        let _ = deferral.Complete();
        r
    }

    fn on_read(&self, args: &GattReadRequestedEventArgs) -> Result<()> {
        let deferral = args.GetDeferral()?;
        let r = (|| -> Result<()> {
            let req = args.GetRequestAsync()?.get()?;
            let key = args.Session()?.DeviceId()?.Id()?.to_string();
            req.RespondWithValue(&make_buffer(&self.status_data(&key))?)?;
            Ok(())
        })();
        let _ = deferral.Complete();
        r
    }

    fn on_subscribers(self: &Arc<Self>, ch: &GattLocalCharacteristic) {
        let Ok(list) = ch.SubscribedClients() else { return };
        let keys: HashSet<String> = list.into_iter().filter_map(|c| client_key(&c)).collect();
        let mut fresh = Vec::new();
        let mut gone: Vec<String> = Vec::new();
        {
            let mut inner = self.inner.lock().unwrap();
            for k in &keys {
                let p = inner.peers.entry(k.clone()).or_insert_with(|| {
                    fresh.push(k.clone());
                    Peer::new()
                });
                p.subscribed = true;
            }
            gone.extend(inner.peers.iter().filter(|(k, p)| p.subscribed && !keys.contains(*k)).map(|(k, _)| k.clone()));
            inner.peers.retain(|k, p| !p.subscribed || keys.contains(k));
        }
        for k in &gone {
            self.link_gone(k);
        }
        for k in fresh {
            self.send_status(&k);
        }
    }

    // packets

    fn reject(&self, why: &str) {
        let n = self.rejected.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n % 100 == 0 {
            log::write(&format!("rejected packet ({n} total): {why}"));
        }
    }

    fn handle(self: &Arc<Self>, key: &str, data: &[u8]) {
        self.packets.fetch_add(1, Ordering::Relaxed);
        if data.is_empty() || data.len() > 80 {
            self.reject(&format!("bad length {}", data.len()));
            return;
        }
        {
            let mut inner = self.inner.lock().unwrap();
            if !inner.peers.contains_key(key) && inner.peers.len() >= 8 {
                drop(inner);
                self.reject("too many peers");
                return;
            }
            inner.peers.entry(key.to_string()).or_insert_with(Peer::new).last_seen = Instant::now();
        }

        match protocol::parse(data) {
            Packet::Reject(why) => self.reject(why),
            Packet::Info { battery, model } => {
                let mut inner = self.inner.lock().unwrap();
                if let Some(p) = inner.peers.get_mut(key) {
                    p.battery = battery;
                    if !model.is_empty() {
                        if p.name == "Unknown device" {
                            p.name = model.clone();
                        }
                        p.model = model;
                    }
                }
            }
            Packet::Hello { device_id, kind: _, name } => {
                let trusted = self.settings.lock().unwrap().trusted.contains(&device_id);
                self.mux.lock().unwrap().hello(&device_id, key);
                let mut ask = false;
                {
                    let mut inner = self.inner.lock().unwrap();
                    // A phone that reconnects over Wi-Fi gets a new socket: forget its dead one at once.
                    if key.starts_with("wifi:") {
                        let stale: Vec<String> = inner
                            .peers
                            .iter()
                            .filter(|(k, p)| k.starts_with("wifi:") && k.as_str() != key && p.device_id.as_deref() == Some(device_id.as_str()))
                            .map(|(k, _)| k.clone())
                            .collect();
                        for k in stale {
                            inner.peers.remove(&k);
                            self.wifi.lock().unwrap().remove(&k);
                        }
                    }
                    match inner.peers.get_mut(key) {
                        Some(p) => {
                            p.device_id = Some(device_id.clone());
                            // Web pages send an empty name (BLE writes can be as short as 20 bytes) and put it in `info`.
                            if !name.is_empty() {
                                p.name = name;
                            } else if !p.model.is_empty() {
                                p.name = p.model.clone();
                            }
                        }
                        None => return,
                    }
                    // One Allow prompt per DEVICE, even when it says hello over Wi-Fi and Bluetooth at once.
                    let decision = if trusted {
                        LinkState::Approved
                    } else if inner.asking.contains(&device_id) {
                        LinkState::Waiting
                    } else if Self::may_ask(&mut inner, &device_id) {
                        ask = true;
                        LinkState::Waiting
                    } else {
                        LinkState::Denied
                    };
                    if let Some(p) = inner.peers.get_mut(key) {
                        p.state = decision;
                    }
                }
                if ask {
                    self.ask_trust(key);
                } else {
                    self.send_status(key);
                }
            }
            Packet::Input(events) => {
                let (approved, dev) = {
                    let inner = self.inner.lock().unwrap();
                    match inner.peers.get(key) {
                        Some(p) => (p.state == LinkState::Approved, p.device_id.clone().unwrap_or_else(|| key.to_string())),
                        None => (false, key.to_string()),
                    }
                };
                if !approved {
                    return;
                }
                // Every link of the device feeds the same device-level stream. The page sends each sample
                // on all its live links; the first copy to arrive wins, so a dying link costs no time.
                for ev in events {
                    let out = {
                        let mut m = self.mux.lock().unwrap();
                        m.link_seen(&dev, key);
                        let e = m.filter(&dev, ev);
                        let paused = m.paused(&dev);
                        e.map(|e| (e, paused))
                    };
                    let Some((ev, paused)) = out else { continue };
                    match ev {
                        Event::Aim { .. } => {
                            self.aims.fetch_add(1, Ordering::Relaxed);
                        }
                        Event::Pause(on) => {
                            for p in self.inner.lock().unwrap().peers.values_mut() {
                                if p.device_id.as_deref() == Some(dev.as_str()) {
                                    p.paused = on;
                                }
                            }
                        }
                        _ => {}
                    }
                    (self.on_event)(&dev, paused, ev);
                }
            }
        }
    }

    /// A link closed. If it was the device's last link and a button was held, let go of it.
    fn link_gone(&self, key: &str) {
        let r = self.mux.lock().unwrap().link_gone(key);
        if let Some((dev, ev)) = r {
            log::write("link lost while the button was held: released");
            (self.on_event)(&dev, false, ev);
        }
    }

    /// At most 3 Allow prompts a minute, and a device that was denied is not asked again for 60 s.
    fn may_ask(inner: &mut Inner, device_id: &str) -> bool {
        let now = Instant::now();
        if let Some(d) = inner.denied_at.get(device_id) {
            if now.duration_since(*d) < Duration::from_secs(60) {
                return false;
            }
        }
        while inner.prompts.front().map(|t| now.duration_since(*t) > Duration::from_secs(60)).unwrap_or(false) {
            inner.prompts.pop_front();
        }
        if inner.prompts.len() >= 3 {
            log::write("pairing prompts rate-limited");
            return false;
        }
        inner.prompts.push_back(now);
        true
    }

    fn ask_trust(self: &Arc<Self>, key: &str) {
        let id = {
            let mut inner = self.inner.lock().unwrap();
            let Some(id) = inner.peers.get(key).and_then(|p| p.device_id.clone()) else { return };
            if inner.asking.contains(&id) {
                return; // another link of this device is already being asked
            }
            inner.asking.insert(id.clone());
            for p in inner.peers.values_mut() {
                if p.device_id.as_deref() == Some(id.as_str()) {
                    p.state = LinkState::Waiting;
                }
            }
            id
        };
        self.send_status(key);
        let me = self.clone();
        std::thread::spawn(move || {
            // give the `info` packet a moment to arrive so the prompt shows a name
            std::thread::sleep(Duration::from_millis(300));
            let name = {
                let inner = me.inner.lock().unwrap();
                inner
                    .peers
                    .values()
                    .filter(|p| p.device_id.as_deref() == Some(id.as_str()))
                    .map(|p| p.name.clone())
                    .find(|n| n != "Unknown device")
                    .unwrap_or_else(|| "Unknown device".to_string())
            };
            let ok = (me.ask)(&name, &validate::code(&id));
            if ok {
                let mut s = me.settings.lock().unwrap();
                if !s.trusted.contains(&id) {
                    s.trusted.push(id.clone());
                    s.save();
                }
            }
            // The answer applies to every link of the device.
            let keys: Vec<String> = {
                let mut inner = me.inner.lock().unwrap();
                inner.asking.remove(&id);
                if !ok {
                    inner.denied_at.insert(id.clone(), Instant::now());
                }
                let mut keys = Vec::new();
                for (k, p) in inner.peers.iter_mut() {
                    if p.device_id.as_deref() == Some(id.as_str()) {
                        p.state = if ok { LinkState::Approved } else { LinkState::Denied };
                        keys.push(k.clone());
                    }
                }
                keys
            };
            log::write(&format!("pairing {}: {name} ({id})", if ok { "allowed" } else { "denied" }));
            for k in keys {
                me.send_status(&k);
            }
        });
    }

    // status

    fn status_data(&self, key: &str) -> Vec<u8> {
        let state = self.inner.lock().unwrap().peers.get(key).map(|p| p.state).unwrap_or(LinkState::Waiting);
        let mut d = vec![state as u8, self.paused.load(Ordering::Relaxed) as u8];
        d.extend(self.pc_name.as_bytes());
        d
    }

    fn send_status(&self, key: &str) {
        if key.starts_with("wifi:") {
            let sink = self.wifi.lock().unwrap().get(key).cloned();
            if let Some(send) = sink {
                send(&self.status_data(key));
            }
            return;
        }
        let ch = self.status.lock().unwrap().as_ref().map(|a| a.0.clone());
        let Some(ch) = ch else { return };
        let Ok(buf) = make_buffer(&self.status_data(key)) else { return };
        if let Ok(list) = ch.SubscribedClients() {
            for c in list {
                if client_key(&c).as_deref() == Some(key) {
                    let _ = ch.NotifyValueForSubscribedClientAsync(&buf, &c);
                }
            }
        }
    }

    fn broadcast_status(&self) {
        let keys: Vec<String> = self.inner.lock().unwrap().peers.keys().cloned().collect();
        for k in keys {
            self.send_status(&k);
        }
    }

    // Wi-Fi link (called by the web server)

    /// A phone opened the Wi-Fi link. False = refuse (too many devices).
    pub fn wifi_connect(self: &Arc<Self>, key: &str, send: Sink) -> bool {
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.peers.len() >= 8 {
                drop(inner);
                self.reject("too many peers");
                return false;
            }
            inner.peers.insert(key.to_string(), Peer::new());
        }
        self.wifi.lock().unwrap().insert(key.to_string(), send);
        self.send_status(key);
        true
    }

    pub fn wifi_packet(self: &Arc<Self>, key: &str, data: &[u8]) {
        self.handle(key, data);
    }

    pub fn wifi_disconnect(&self, key: &str) {
        self.wifi.lock().unwrap().remove(key);
        self.inner.lock().unwrap().peers.remove(key);
        self.link_gone(key);
    }

    /// Devices that have introduced themselves, for the control panel. One row per device;
    /// `wifi` / `ble` say which links of it are up.
    pub fn peer_infos(&self) -> Vec<PeerInfo> {
        let inner = self.inner.lock().unwrap();
        let mut by_dev: HashMap<String, PeerInfo> = HashMap::new();
        for (k, p) in inner.peers.iter() {
            let Some(id) = p.device_id.as_ref() else { continue };
            let is_wifi = k.starts_with("wifi:");
            let seen_ms = p.last_seen.elapsed().as_millis() as u64;
            let rank = |s: LinkState| match s {
                LinkState::Approved => 2,
                LinkState::Waiting => 1,
                LinkState::Denied => 0,
            };
            match by_dev.get_mut(id) {
                Some(e) => {
                    e.wifi |= is_wifi;
                    e.ble |= !is_wifi;
                    if rank(p.state) > rank(e.state) {
                        e.state = p.state;
                    }
                    e.paused |= p.paused;
                    if seen_ms < e.seen_ms {
                        e.seen_ms = seen_ms;
                        e.battery = p.battery.or(e.battery);
                    } else if e.battery.is_none() {
                        e.battery = p.battery;
                    }
                }
                None => {
                    by_dev.insert(
                        id.clone(),
                        PeerInfo {
                            name: p.name.clone(),
                            model: p.model.clone(),
                            battery: p.battery,
                            state: p.state,
                            paused: p.paused,
                            wifi: is_wifi,
                            ble: !is_wifi,
                            seen_ms,
                            code: validate::code(id),
                        },
                    );
                }
            }
        }
        let mut v: Vec<PeerInfo> = by_dev.into_values().collect();
        v.sort_by_key(|p| p.seen_ms);
        v
    }

    pub fn trusted_count(&self) -> usize {
        self.settings.lock().unwrap().trusted.len()
    }

    // queries / actions for the tray

    pub fn set_paused(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
        self.broadcast_status();
    }

    /// (name, battery) of the most recently seen approved device, if seen in the last 6 s.
    pub fn active_peer(&self) -> Option<(String, Option<u8>)> {
        let inner = self.inner.lock().unwrap();
        inner
            .peers
            .values()
            .filter(|p| p.state == LinkState::Approved && p.last_seen.elapsed() < Duration::from_secs(6))
            .max_by_key(|p| std::cmp::Reverse(p.last_seen.elapsed()))
            .map(|p| (p.name.clone(), p.battery))
    }

    pub fn forget_all(&self) {
        {
            let mut s = self.settings.lock().unwrap();
            s.trusted.clear();
            s.save();
        }
        for p in self.inner.lock().unwrap().peers.values_mut() {
            p.state = LinkState::Denied;
        }
        self.broadcast_status();
    }
}

#[cfg(test)]
mod glue_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn hello(id: u8) -> Vec<u8> {
        let mut v = vec![1u8];
        v.extend([id; 16]);
        v.push(1);
        v.extend(b"Phone");
        v
    }
    fn aim(t: f64) -> Vec<u8> {
        let mut v = vec![2u8];
        v.extend(t.to_le_bytes());
        v.extend(1.0f32.to_le_bytes());
        v.extend(2.0f32.to_le_bytes());
        v
    }
    fn timed(kind: u8, t: f64) -> Vec<u8> {
        let mut v = vec![kind];
        v.extend(t.to_le_bytes());
        v
    }

    type Log = Arc<Mutex<Vec<(String, Event)>>>;
    fn rig(auto_allow: bool) -> (Arc<Receiver>, Log, Arc<AtomicUsize>) {
        let log: Log = Arc::new(Mutex::new(vec![]));
        let asks = Arc::new(AtomicUsize::new(0));
        let (l2, a2) = (log.clone(), asks.clone());
        let r = Receiver::new(
            Arc::new(Mutex::new(Settings::default())),
            Box::new(move |k, _p, e| l2.lock().unwrap().push((k.to_string(), e))),
            Box::new(move |_n, _c| {
                a2.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(200)); // user is slow: both hellos arrive meanwhile
                auto_allow
            }),
        );
        (r, log, asks)
    }
    fn state(r: &Arc<Receiver>, key: &str) -> LinkState {
        r.inner.lock().unwrap().peers.get(key).map(|p| p.state).unwrap()
    }

    #[test]
    fn one_prompt_for_two_links_and_both_approved() {
        let (r, _log, asks) = rig(true);
        assert!(r.wifi_connect("wifi:1", Arc::new(|_| {})));
        r.handle("wifi:1", &hello(7));
        r.handle("ble", &hello(7)); // same device over Bluetooth, before the user answered
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(asks.load(Ordering::SeqCst), 1, "must prompt once per device");
        assert_eq!(state(&r, "wifi:1"), LinkState::Approved);
        assert_eq!(state(&r, "ble"), LinkState::Approved);
        let infos = r.peer_infos();
        assert_eq!(infos.len(), 1, "one panel row per device");
        assert!(infos[0].wifi && infos[0].ble);
    }

    #[test]
    fn denied_applies_to_both_links() {
        let (r, _log, asks) = rig(false);
        r.wifi_connect("wifi:1", Arc::new(|_| {}));
        r.handle("wifi:1", &hello(7));
        r.handle("ble", &hello(7));
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(asks.load(Ordering::SeqCst), 1);
        assert_eq!(state(&r, "wifi:1"), LinkState::Denied);
        assert_eq!(state(&r, "ble"), LinkState::Denied);
    }

    fn approved_pair() -> (Arc<Receiver>, Log) {
        let (r, log, _a) = rig(true);
        r.settings.lock().unwrap().trusted.push(validate::clean("07070707-0707-0707-0707-070707070707", 64));
        r.wifi_connect("wifi:1", Arc::new(|_| {}));
        r.handle("wifi:1", &hello(7));
        r.handle("ble", &hello(7));
        (r, log)
    }

    #[test]
    fn same_sample_on_both_links_moves_once_and_key_is_the_device() {
        let (r, log) = approved_pair();
        assert_eq!(state(&r, "ble"), LinkState::Approved);
        r.handle("wifi:1", &aim(1.0));
        r.handle("ble", &aim(1.0));
        r.handle("ble", &aim(1.01));
        r.handle("wifi:1", &aim(1.01));
        let l = log.lock().unwrap();
        assert_eq!(l.len(), 2);
        assert!(l.iter().all(|(k, _)| k == "07070707-0707-0707-0707-070707070707"), "keyed by device, not link: {:?}", l[0].0);
    }

    #[test]
    fn click_on_both_links_fires_once_and_dead_link_does_not_stick() {
        let (r, log) = approved_pair();
        r.handle("wifi:1", &timed(3, 5.0));
        r.handle("ble", &timed(3, 5.0));
        assert_eq!(log.lock().unwrap().iter().filter(|(_, e)| matches!(e, Event::Press(_))).count(), 1);
        r.wifi_disconnect("wifi:1"); // BLE still alive: stays held
        assert!(!log.lock().unwrap().iter().any(|(_, e)| matches!(e, Event::Release(_))));
        r.handle("ble", &timed(4, 5.5));
        assert_eq!(log.lock().unwrap().iter().filter(|(_, e)| matches!(e, Event::Release(_))).count(), 1);
        // hold again, then lose the only link
        r.handle("ble", &timed(3, 9.0));
        r.link_gone("ble");
        assert_eq!(log.lock().unwrap().iter().filter(|(_, e)| matches!(e, Event::Release(_))).count(), 2);
    }

    #[test]
    fn wifi_reconnect_replaces_the_dead_socket() {
        let (r, _log) = approved_pair();
        r.wifi_connect("wifi:2", Arc::new(|_| {}));
        r.handle("wifi:2", &hello(7));
        let keys: Vec<String> = r.inner.lock().unwrap().peers.keys().cloned().collect();
        assert!(!keys.contains(&"wifi:1".to_string()), "{keys:?}");
        assert_eq!(r.peer_infos().len(), 1);
    }
}
