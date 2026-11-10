//! The PC side of the Bluetooth link: advertises one GATT service the phone page connects to.
//! Nothing goes over Wi-Fi and there is no server.
use super::log;
use super::settings::Settings;
use airwand::protocol::{self, Event, LinkState, Packet};
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
    last_aim_t: f64,
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
            last_aim_t: f64::NEG_INFINITY,
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
        {
            let mut inner = self.inner.lock().unwrap();
            for k in &keys {
                let p = inner.peers.entry(k.clone()).or_insert_with(|| {
                    fresh.push(k.clone());
                    Peer::new()
                });
                p.subscribed = true;
            }
            inner.peers.retain(|k, p| !p.subscribed || keys.contains(k));
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
                let mut ask = false;
                {
                    let mut inner = self.inner.lock().unwrap();
                    let Some(p) = inner.peers.get_mut(key) else { return };
                    p.device_id = Some(device_id.clone());
                    // Web pages send an empty name (BLE writes can be as short as 20 bytes) and put it in `info`.
                    if !name.is_empty() {
                        p.name = name;
                    } else if !p.model.is_empty() {
                        p.name = p.model.clone();
                    }
                    if trusted {
                        p.state = LinkState::Approved;
                    } else if Self::may_ask(&mut inner, &device_id) {
                        ask = true;
                    } else {
                        if let Some(p) = inner.peers.get_mut(key) {
                            p.state = LinkState::Denied;
                        }
                    }
                }
                if ask {
                    self.ask_trust(key);
                } else {
                    self.send_status(key);
                }
            }
            Packet::Input(events) => {
                let (approved, paused) = {
                    let inner = self.inner.lock().unwrap();
                    match inner.peers.get(key) {
                        Some(p) => (p.state == LinkState::Approved, p.paused),
                        None => (false, false),
                    }
                };
                if !approved {
                    return;
                }
                // Wi-Fi is faster than Bluetooth: while the same device is also live over Wi-Fi, ignore its Bluetooth input.
                if !key.starts_with("wifi:") && self.wifi_preferred(key) {
                    return;
                }
                let mut paused = paused;
                for ev in events {
                    match ev {
                        Event::Aim { t, az, el } => {
                            // Newest wins: batched packets repeat the previous sample.
                            let fresh = {
                                let mut inner = self.inner.lock().unwrap();
                                match inner.peers.get_mut(key) {
                                    Some(p) if !(t <= p.last_aim_t && p.last_aim_t - t < 5.0) => {
                                        p.last_aim_t = t;
                                        true
                                    }
                                    _ => false,
                                }
                            };
                            if fresh {
                                self.aims.fetch_add(1, Ordering::Relaxed);
                                (self.on_event)(key, paused, Event::Aim { t, az, el });
                            }
                        }
                        Event::Pause(on) => {
                            paused = on;
                            if let Some(p) = self.inner.lock().unwrap().peers.get_mut(key) {
                                p.paused = on;
                            }
                            (self.on_event)(key, paused, Event::Pause(on));
                        }
                        other => (self.on_event)(key, paused, other),
                    }
                }
            }
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
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.asking.contains(key) || !inner.peers.contains_key(key) {
                return;
            }
            inner.asking.insert(key.to_string());
            if let Some(p) = inner.peers.get_mut(key) {
                p.state = LinkState::Waiting;
            }
        }
        self.send_status(key);
        let me = self.clone();
        let key = key.to_string();
        std::thread::spawn(move || {
            // give the `info` packet a moment to arrive so the prompt shows a name
            std::thread::sleep(Duration::from_millis(300));
            let (name, id) = {
                let inner = me.inner.lock().unwrap();
                match inner.peers.get(&key) {
                    Some(p) => (p.name.clone(), p.device_id.clone().unwrap_or_default()),
                    None => return,
                }
            };
            let ok = (me.ask)(&name, &validate::code(&id));
            if ok {
                let mut s = me.settings.lock().unwrap();
                if !s.trusted.contains(&id) {
                    s.trusted.push(id.clone());
                    s.save();
                }
            }
            {
                let mut inner = me.inner.lock().unwrap();
                inner.asking.remove(&key);
                if let Some(p) = inner.peers.get_mut(&key) {
                    p.state = if ok { LinkState::Approved } else { LinkState::Denied };
                }
                if !ok {
                    inner.denied_at.insert(id.clone(), Instant::now());
                }
            }
            log::write(&format!("pairing {}: {name} ({id})", if ok { "allowed" } else { "denied" }));
            me.send_status(&key);
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

    /// True when the device behind this Bluetooth key is also approved and active over Wi-Fi.
    fn wifi_preferred(&self, key: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        let Some(id) = inner.peers.get(key).and_then(|p| p.device_id.clone()) else { return false };
        inner.peers.iter().any(|(k, p)| {
            k.starts_with("wifi:")
                && p.state == LinkState::Approved
                && p.device_id.as_deref() == Some(id.as_str())
                && p.last_seen.elapsed() < Duration::from_secs(2)
        })
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
        let mut inner = self.inner.lock().unwrap();
        inner.peers.remove(key);
        inner.asking.remove(key);
    }

    /// Devices that have introduced themselves, for the control panel.
    pub fn peer_infos(&self) -> Vec<PeerInfo> {
        let inner = self.inner.lock().unwrap();
        let mut v: Vec<PeerInfo> = inner
            .peers
            .iter()
            .filter_map(|(k, p)| {
                let id = p.device_id.as_ref()?;
                Some(PeerInfo {
                    name: p.name.clone(),
                    model: p.model.clone(),
                    battery: p.battery,
                    state: p.state,
                    paused: p.paused,
                    wifi: k.starts_with("wifi:"),
                    seen_ms: p.last_seen.elapsed().as_millis() as u64,
                    code: validate::code(id),
                })
            })
            .collect();
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
