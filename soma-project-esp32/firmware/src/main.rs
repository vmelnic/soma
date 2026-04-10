// soma-esp32-firmware — composes the hardware port crates into a flashable
// binary for either ESP32-S3 or ESP32 (Xtensa LX6, e.g. WROOM-32D).
//
// The chip is selected by exactly one cargo feature: chip-esp32s3 (default)
// or chip-esp32. The pin assignments below are chip-specific because the
// available GPIO range and module flash reservations differ between chips.
//
// === ESP32-S3 (Sunton 1732S019 dev board) ===========================
//   GPIO15  — gpio.write/read/toggle test pin (no LED on this board)
//   GPIO8   — I2C SDA
//   GPIO9   — I2C SCL
//   GPIO35  — SPI3 SCK   (off the LCD's SPI2)
//   GPIO36  — SPI3 MOSI
//   GPIO2   — ADC1_CH1
//   GPIO16  — LEDC channel 0 (PWM)
//   GPIO17  — UART1 TX
//   GPIO18  — UART1 RX
//
//   The 1732S019 wires its ST7789 LCD to GPIO 1, 4-7, 10-14, 38-41 — this
//   firmware does NOT touch those pins; the LCD stays in reset.
//
// === ESP32 / ESP-WROOM-32D ==========================================
//   GPIO13  — gpio.write/read/toggle test pin
//   GPIO21  — I2C SDA  (default ESP32 I2C0 pins)
//   GPIO22  — I2C SCL
//   GPIO18  — SPI3 (VSPI) SCK
//   GPIO23  — SPI3 (VSPI) MOSI
//   GPIO34  — ADC1_CH6 (input-only — fine for ADC)
//   GPIO25  — LEDC channel 0 (PWM)
//   GPIO16  — UART1 TX
//   GPIO17  — UART1 RX
//
//   GPIO 6-11 on the WROOM-32D are wired to the internal QSPI flash —
//   touching them bricks the boot. Strapping pins (0, 2, 5, 12, 15) are
//   avoided to keep the boot reliable.
//
// Host transport for both chips: UART0 at 115200 8N1, on the same TX/RX
// pins the on-board USB-to-UART bridge (CH340 / CP210x) is wired to.
// esp-println shares this UART, so the host parser must skip log lines
// until it finds a 4-byte length prefix followed by valid JSON.

#![no_std]
#![no_main]

extern crate alloc;

mod chip;
#[cfg(feature = "wifi")]
mod mdns;

#[cfg(feature = "wifi")]
use core::sync::atomic::{AtomicU32, Ordering};

// Lock-free shared state: the DHCP-assigned IPv4 address as a u32
// (big-endian byte order packed: [a, b, c, d] -> (a << 24) | (b << 16) |
// (c << 8) | d). 0 means "no address assigned".
//
// Written by TcpListenerState on DhcpEvent::Configured/Deconfigured.
// Read by RealWifiOps::status so wifi.status can report the real IP.
//
// AtomicU32 is no_std-friendly, allocation-free, and doesn't need a
// critical section for 4-byte reads/writes on our Xtensa + RISC-V
// targets — both chips have atomic u32 support.
#[cfg(feature = "wifi")]
static ASSIGNED_IPV4: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "wifi")]
fn set_assigned_ipv4(addr: smoltcp::wire::Ipv4Address) {
    let b = addr.octets();
    let packed = ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32);
    ASSIGNED_IPV4.store(packed, Ordering::Relaxed);
}

#[cfg(feature = "wifi")]
fn clear_assigned_ipv4() {
    ASSIGNED_IPV4.store(0, Ordering::Relaxed);
}

#[cfg(feature = "wifi")]
fn get_assigned_ipv4_string() -> Option<alloc::string::String> {
    let packed = ASSIGNED_IPV4.load(Ordering::Relaxed);
    if packed == 0 {
        return None;
    }
    let b = [
        ((packed >> 24) & 0xFF) as u8,
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    ];
    Some(alloc::format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]))
}

use alloc::string::ToString;

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    main,
    uart::Uart,
};
#[cfg(feature = "wifi")]
use alloc::boxed::Box;
#[cfg(feature = "wifi")]
use esp_hal::{rng::Rng, timer::timg::TimerGroup};
#[cfg(feature = "wifi")]
use esp_wifi::{
    init as esp_wifi_init,
    wifi::{
        utils::create_network_interface, AuthMethod, ClientConfiguration,
        Configuration as WifiConfiguration, WifiController, WifiDevice, WifiStaDevice,
    },
    EspWifiController,
};

// ---------------------------------------------------------------------------
// ChipBoot — what each chip module hands back to main() after register_all_ports
// ---------------------------------------------------------------------------
//
// The host_uart is always present. The wifi fields are present only when the
// wifi cargo feature is enabled, and only if esp-wifi initialization
// succeeded — they're Option<&'static mut> so the dispatch loop can tell
// "wifi compiled in but radio failed at boot" apart from "wifi compiled in
// and ready to listen". Without the wifi feature these fields don't exist
// at all, so the firmware doesn't pay any code-size cost for them.
pub struct ChipBoot {
    pub host_uart: Uart<'static, esp_hal::Blocking>,
    #[cfg(feature = "wifi")]
    pub wifi_iface: Option<&'static mut Interface>,
    #[cfg(feature = "wifi")]
    pub wifi_device: Option<&'static mut WifiDevice<'static, WifiStaDevice>>,
    #[cfg(feature = "wifi")]
    pub wifi_controller: Option<&'static mut WifiController<'static>>,
}

// ---------------------------------------------------------------------------
// init_wifi_subsystem — chip-agnostic wifi/smoltcp setup
// ---------------------------------------------------------------------------
//
// esp-wifi 0.12's API is the same across every Espressif chip with a radio.
// The setup is: TIMG0 -> esp_wifi::init -> create_network_interface ->
// (Interface, WifiDevice, WifiController). All three are leaked to 'static
// so they can live in the dispatch loop's TcpListenerState. Returns Nones
// if any step fails — the firmware still boots and the UART transport still
// works.
//
// This function is called from each chip module's register_all_ports BEFORE
// it consumes the GPIO/UART/etc peripheral fields, so the wifi-related
// fields (TIMG0, RNG, RADIO_CLK, WIFI) get moved out first.
#[cfg(feature = "wifi")]
pub fn init_wifi_subsystem(
    timg0_peripheral: esp_hal::peripherals::TIMG0,
    rng_peripheral: esp_hal::peripherals::RNG,
    radio_clk: esp_hal::peripherals::RADIO_CLK,
    wifi_peripheral: esp_hal::peripherals::WIFI,
) -> (
    Option<&'static mut Interface>,
    Option<&'static mut WifiDevice<'static, WifiStaDevice>>,
    Option<&'static mut WifiController<'static>>,
) {
    let timg0 = TimerGroup::new(timg0_peripheral);
    let rng = Rng::new(rng_peripheral);
    let init_result = esp_wifi_init(timg0.timer0, rng, radio_clk);

    match init_result {
        Ok(init) => {
            let init_static: &'static EspWifiController<'static> =
                Box::leak(Box::new(init));
            println!("[wifi] esp-wifi initialized");
            match create_network_interface(init_static, wifi_peripheral, WifiStaDevice) {
                Ok((iface, device, controller)) => {
                    println!("[wifi] smoltcp Interface created");
                    (
                        Some(Box::leak(Box::new(iface))),
                        Some(Box::leak(Box::new(device))),
                        Some(Box::leak(Box::new(controller))),
                    )
                }
                Err(e) => {
                    println!("[wifi] create_network_interface failed: {:?}", e);
                    (None, None, None)
                }
            }
        }
        Err(e) => {
            println!(
                "[wifi] esp-wifi init failed: {:?} (wifi port will use storage-only backend)",
                e
            );
            (None, None, None)
        }
    }
}

#[cfg(feature = "wifi")]
use smoltcp::iface::{Interface, SocketSet, SocketStorage};
#[cfg(feature = "wifi")]
use smoltcp::socket::dhcpv4::{Event as DhcpEvent, Socket as DhcpSocket};
#[cfg(feature = "wifi")]
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer};
#[cfg(feature = "wifi")]
use smoltcp::wire::{IpCidr, Ipv4Address, Ipv4Cidr};
use esp_println::println;
use serde_json::json;

use soma_esp32_leaf::{
    decode_frame, encode_response, CompositeDispatcher, FrameError, LeafState, Routine,
    RoutineStep, SkillDispatcher, TransportMessage, TransportResponse, DEFAULT_MAX_FRAME,
};

// ESP-IDF application image descriptor. The chip's stage-2 bootloader reads
// this 256-byte struct from `.rodata_desc.appdesc` to validate the image
// (magic word, min/max eFuse block revisions, MMU page size). Without it,
// the bootloader reads garbage and refuses to boot ("Image requires efuse
// blk rev >= v237.62" or similar).
//
// Layout matches `esp_app_desc_t` from ESP-IDF v5.x:
//   u32 magic_word         = 0xABCD5432
//   u32 secure_version     = 0
//   u32 reserv1[2]         = 0
//   char version[32]
//   char project_name[32]
//   char time[16]
//   char date[16]
//   char idf_ver[32]
//   u8 app_elf_sha256[32]  (zero — espflash patches this at flash time)
//   u16 min_efuse_blk_rev_full = 0      (any chip OK)
//   u16 max_efuse_blk_rev_full = 0xFFFF (no upper bound)
//   u8 mmu_page_size       = 16  (log2 of 64 KB — S3 default)
//   u8 reserv3[3]          = 0
//   u32 reserv2[18]        = 0
//
// Hand-rolled rather than depending on esp-bootloader-esp-idf because that
// crate transitively pulls in esp-rom-sys which collides with esp-hal 0.23
// on `rtc_clk_xtal_freq_get`.
#[repr(C)]
struct EspAppDesc {
    magic_word: u32,
    secure_version: u32,
    reserv1: [u32; 2],
    version: [u8; 32],
    project_name: [u8; 32],
    time: [u8; 16],
    date: [u8; 16],
    idf_ver: [u8; 32],
    app_elf_sha256: [u8; 32],
    min_efuse_blk_rev_full: u16,
    max_efuse_blk_rev_full: u16,
    mmu_page_size: u8,
    reserv3: [u8; 3],
    reserv2: [u32; 18],
}

const fn pad_str<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && i < N - 1 {
        out[i] = bytes[i];
        i += 1;
    }
    out
}

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".rodata_desc.appdesc")]
static esp_app_desc: EspAppDesc = EspAppDesc {
    magic_word: 0xABCD_5432,
    secure_version: 0,
    reserv1: [0; 2],
    version: pad_str::<32>("0.1.0"),
    project_name: pad_str::<32>("soma-esp32-firmware"),
    time: pad_str::<16>("00:00:00"),
    date: pad_str::<16>("Apr 10 2026"),
    idf_ver: pad_str::<32>("v5.5.0"),
    app_elf_sha256: [0; 32],
    min_efuse_blk_rev_full: 0,
    max_efuse_blk_rev_full: u16::MAX,
    mmu_page_size: 16,
    reserv3: [0; 3],
    reserv2: [0; 18],
};

// FlashKvStore: a tiny key-value store backed by one SPI flash sector via
// esp-storage. Used by the storage port to persist WiFi credentials and any
// other config across reboots.
//
// Format: a single 4 KB sector at SECTOR_OFFSET. The sector contains a magic
// header followed by length-prefixed key/value records. On every set/delete,
// the entire sector is rewritten (slow but simple, and writes are rare).
//
// Layout:
//   [4 bytes magic "SOMA"]
//   [2 bytes record_count (LE)]
//   for each record:
//     [2 bytes key_len (LE)]
//     [key bytes]
//     [2 bytes value_len (LE)]
//     [value bytes]

use embedded_storage::{ReadStorage, Storage};
use esp_storage::FlashStorage;
use soma_esp32_port_storage::{KvStore, StorageError};

struct FlashKvStore {
    flash: FlashStorage,
    cache: alloc::collections::BTreeMap<alloc::string::String, alloc::string::String>,
    loaded: bool,
}

impl FlashKvStore {
    /// Flash offset where the SOMA config sector lives.
    ///
    /// MUST be past the end of the application's .text / .rodata segments,
    /// otherwise writes here overwrite app code and the next boot fails
    /// with `esp_image: Checksum failed`. With the wifi feature on, the
    /// firmware image is ~700 KB, so its segments reach ~0xBF000 absolute
    /// flash offset (app partition starts at 0x10000). 0x3F_F000 is the
    /// last 4 KB sector of the default 4 MB espflash partition table
    /// (nvs=0x9000..0xF000, phy_init=0xF000..0x10000, factory=0x10000..0x400000),
    /// guaranteed to be well past any reasonable app size on both ESP32
    /// (4 MB flash) and ESP32-S3 (16 MB flash) with the same partition
    /// table. Adjust only if your partition table changes.
    const SECTOR_OFFSET: u32 = 0x3F_F000;
    const SECTOR_SIZE: usize = 4096;
    const MAGIC: &'static [u8; 4] = b"SOMA";

    fn new() -> Self {
        let mut s = Self {
            flash: FlashStorage::new(),
            cache: alloc::collections::BTreeMap::new(),
            loaded: false,
        };
        // Best-effort load on construction. Errors leave the cache empty —
        // the next set() will write a fresh sector.
        let _ = s.load();
        s
    }

    fn load(&mut self) -> Result<(), StorageError> {
        let mut buf = [0u8; Self::SECTOR_SIZE];
        self.flash
            .read(Self::SECTOR_OFFSET, &mut buf)
            .map_err(|_| StorageError::BackendError("flash read failed".to_string()))?;

        if &buf[..4] != Self::MAGIC {
            // Sector hasn't been initialized yet (likely 0xFF from erased flash).
            self.loaded = true;
            return Ok(());
        }

        let count = u16::from_le_bytes([buf[4], buf[5]]) as usize;
        let mut pos = 6;
        for _ in 0..count {
            if pos + 2 > Self::SECTOR_SIZE {
                break;
            }
            let key_len = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
            pos += 2;
            if pos + key_len > Self::SECTOR_SIZE {
                break;
            }
            let key = match core::str::from_utf8(&buf[pos..pos + key_len]) {
                Ok(s) => s.to_string(),
                Err(_) => break,
            };
            pos += key_len;
            if pos + 2 > Self::SECTOR_SIZE {
                break;
            }
            let val_len = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
            pos += 2;
            if pos + val_len > Self::SECTOR_SIZE {
                break;
            }
            let val = match core::str::from_utf8(&buf[pos..pos + val_len]) {
                Ok(s) => s.to_string(),
                Err(_) => break,
            };
            pos += val_len;
            self.cache.insert(key, val);
        }

        self.loaded = true;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        let mut buf = [0xFFu8; Self::SECTOR_SIZE];
        buf[..4].copy_from_slice(Self::MAGIC);
        let count = self.cache.len() as u16;
        buf[4..6].copy_from_slice(&count.to_le_bytes());
        let mut pos = 6;
        for (k, v) in &self.cache {
            let kb = k.as_bytes();
            let vb = v.as_bytes();
            let needed = 2 + kb.len() + 2 + vb.len();
            if pos + needed > Self::SECTOR_SIZE {
                return Err(StorageError::OutOfSpace);
            }
            buf[pos..pos + 2].copy_from_slice(&(kb.len() as u16).to_le_bytes());
            pos += 2;
            buf[pos..pos + kb.len()].copy_from_slice(kb);
            pos += kb.len();
            buf[pos..pos + 2].copy_from_slice(&(vb.len() as u16).to_le_bytes());
            pos += 2;
            buf[pos..pos + vb.len()].copy_from_slice(vb);
            pos += vb.len();
        }

        self.flash
            .write(Self::SECTOR_OFFSET, &buf)
            .map_err(|_| StorageError::BackendError("flash write failed".to_string()))?;
        Ok(())
    }
}

impl KvStore for FlashKvStore {
    fn get(&self, key: &str) -> Result<Option<alloc::string::String>, StorageError> {
        Ok(self.cache.get(key).cloned())
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        self.cache.insert(key.to_string(), value.to_string());
        self.flush()
    }

    fn delete(&mut self, key: &str) -> Result<bool, StorageError> {
        let removed = self.cache.remove(key).is_some();
        if removed {
            self.flush()?;
        }
        Ok(removed)
    }

    fn list(
        &self,
        prefix: Option<&str>,
    ) -> Result<alloc::vec::Vec<alloc::string::String>, StorageError> {
        Ok(self
            .cache
            .keys()
            .filter(|k| match prefix {
                Some(p) => k.starts_with(p),
                None => true,
            })
            .cloned()
            .collect())
    }

    fn clear(&mut self) -> Result<(), StorageError> {
        self.cache.clear();
        self.flush()
    }
}

// ---------------------------------------------------------------------------
// Wifi backends — RealWifiOps and StorageOnlyWifiOps (gated on `wifi` feature)
// ---------------------------------------------------------------------------
//
// RealWifiOps drives the actual esp-wifi radio. It owns a 'static-lifetime
// reference to the WifiController (leaked at boot from main()).
//
// StorageOnlyWifiOps is the fallback used when esp-wifi fails to initialize
// (or when wifi feature is disabled). It persists credentials to FlashKvStore
// but never touches the radio. Useful for build/dev scenarios where the
// firmware should boot even without WiFi support.

#[cfg(feature = "wifi")]
use soma_esp32_port_wifi::{WifiError, WifiNetwork, WifiOps, WifiState};

#[cfg(feature = "wifi")]
struct RealWifiOps {
    controller: &'static mut WifiController<'static>,
    store: FlashKvStore,
}

#[cfg(feature = "wifi")]
impl WifiOps for RealWifiOps {
    fn scan(&mut self) -> Result<alloc::vec::Vec<WifiNetwork>, WifiError> {
        // Bring the radio up if it isn't already. start() is idempotent in
        // the sense that it errors with AlreadyStarted, which we ignore.
        if let Err(e) = self.controller.start() {
            // Tolerate "already started" — anything else is a real failure.
            let msg = alloc::format!("{:?}", e);
            if !msg.contains("Already") && !msg.contains("started") {
                return Err(WifiError::HardwareError(msg));
            }
        }

        match self.controller.scan_n::<16>() {
            Ok((aps, _count)) => {
                let networks: alloc::vec::Vec<WifiNetwork> = aps
                    .iter()
                    .map(|ap| WifiNetwork {
                        ssid: ap.ssid.as_str().to_string(),
                        rssi: ap.signal_strength as i32,
                        security: alloc::format!("{:?}", ap.auth_method),
                        channel: ap.channel,
                    })
                    .collect();
                Ok(networks)
            }
            Err(e) => Err(WifiError::HardwareError(alloc::format!("{:?}", e))),
        }
    }

    fn configure(&mut self, ssid: &str, password: &str) -> Result<(), WifiError> {
        // Persist to flash so the next boot picks them up automatically.
        self.store
            .set("wifi.ssid", ssid)
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;
        self.store
            .set("wifi.password", password)
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;

        // Apply immediately to the running controller too.
        let mut hp_ssid: heapless::String<32> = heapless::String::new();
        hp_ssid
            .push_str(ssid)
            .map_err(|_| WifiError::HardwareError("ssid too long".to_string()))?;
        let mut hp_pw: heapless::String<64> = heapless::String::new();
        hp_pw
            .push_str(password)
            .map_err(|_| WifiError::HardwareError("password too long".to_string()))?;

        let client_config = ClientConfiguration {
            ssid: hp_ssid,
            password: hp_pw,
            auth_method: if password.is_empty() {
                AuthMethod::None
            } else {
                AuthMethod::WPA2Personal
            },
            ..Default::default()
        };

        self.controller
            .set_configuration(&WifiConfiguration::Client(client_config))
            .map_err(|e| WifiError::HardwareError(alloc::format!("{:?}", e)))?;

        // Make sure the radio is started, then connect.
        let _ = self.controller.start();
        self.controller
            .connect()
            .map_err(|e| match alloc::format!("{:?}", e).as_str() {
                s if s.contains("Auth") => WifiError::AuthFailed,
                s if s.contains("NotFound") => WifiError::NoApFound,
                s => WifiError::HardwareError(s.to_string()),
            })?;

        // connect() returns OK as soon as the association request has been
        // *sent*, not when it's completed. Poll is_connected() until it
        // actually becomes true or the timeout expires. Without this wait,
        // the caller sees wifi.configure succeed but the radio link isn't
        // up yet — smoltcp's DhcpSocket polls the link immediately after
        // and sends DISCOVER into the void, times out, and never retries.
        let delay = Delay::new();
        let deadline_polls = 200; // 200 * 50ms = 10s max wait
        let mut polls = 0;
        loop {
            if self.controller.is_connected().unwrap_or(false) {
                break;
            }
            polls += 1;
            if polls >= deadline_polls {
                return Err(WifiError::HardwareError(
                    "connect() returned OK but is_connected() never became true".to_string(),
                ));
            }
            delay.delay_millis(50);
        }
        Ok(())
    }

    fn status(&self) -> Result<WifiState, WifiError> {
        let connected = self.controller.is_connected().unwrap_or(false);
        let ssid = self
            .store
            .get("wifi.ssid")
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;
        // Read the DHCP-assigned IPv4 address from the lock-free
        // ASSIGNED_IPV4 atomic that TcpListenerState writes on
        // DhcpEvent::Configured. None until DHCP finishes.
        let ip = get_assigned_ipv4_string();
        Ok(WifiState {
            connected,
            ssid,
            ip,
            rssi: None,
            mac: None,
        })
    }

    fn disconnect(&mut self) -> Result<(), WifiError> {
        self.controller
            .disconnect()
            .map_err(|e| WifiError::HardwareError(alloc::format!("{:?}", e)))?;
        Ok(())
    }

    fn forget(&mut self) -> Result<(), WifiError> {
        self.store
            .delete("wifi.ssid")
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;
        self.store
            .delete("wifi.password")
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;
        let _ = self.controller.disconnect();
        Ok(())
    }
}

/// Fallback when esp-wifi failed to initialize: just persists credentials,
/// no radio access. The wifi.scan primitive returns an empty list and
/// wifi.status reports `connected: false`.
#[cfg(feature = "wifi")]
struct StorageOnlyWifiOps {
    store: FlashKvStore,
}

#[cfg(feature = "wifi")]
impl WifiOps for StorageOnlyWifiOps {
    fn scan(&mut self) -> Result<alloc::vec::Vec<WifiNetwork>, WifiError> {
        Ok(alloc::vec::Vec::new())
    }
    fn configure(&mut self, ssid: &str, password: &str) -> Result<(), WifiError> {
        self.store
            .set("wifi.ssid", ssid)
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;
        self.store
            .set("wifi.password", password)
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;
        Ok(())
    }
    fn status(&self) -> Result<WifiState, WifiError> {
        let ssid = self
            .store
            .get("wifi.ssid")
            .map_err(|e| WifiError::StorageError(alloc::format!("{:?}", e)))?;
        Ok(WifiState {
            connected: false,
            ssid,
            ip: None,
            rssi: None,
            mac: None,
        })
    }
    fn disconnect(&mut self) -> Result<(), WifiError> {
        Ok(())
    }
    fn forget(&mut self) -> Result<(), WifiError> {
        let _ = self.store.delete("wifi.ssid");
        let _ = self.store.delete("wifi.password");
        Ok(())
    }
}

#[main]
fn main() -> ! {
    // Force-link the esp_app_desc static. `#[used]` should keep it but
    // black_box is belt-and-suspenders against aggressive LTO.
    core::hint::black_box(&esp_app_desc);

    // 1. Heap
    //
    // esp-wifi needs heap room for packet buffers. 64 KB leaves ~256 KB
    // of SRAM for esp-wifi's static buffers, the leaf state, the receive
    // buffer, and stacks.
    // Heap size matters with wifi enabled. esp-wifi 0.12 needs ~72 KB for
    // its internal buffers + smoltcp sockets + TCP rx/tx + our vecs.
    // 64 KB was tight and left no headroom; 96 KB has ~24 KB spare after
    // init on ESP32-S3 with the default port set. Without wifi, 96 KB is
    // still fine — the extra headroom is free.
    esp_alloc::heap_allocator!(96 * 1024);

    // 2. Chip init — peripherals + clock. The active chip module knows how
    //    to set up its own clock; main.rs stays chip-agnostic.
    let peripherals = chip::active::init_peripherals();

    println!();
    println!("=========================================");
    println!("SOMA {} leaf firmware booted", chip::active::NAME);
    println!("Free heap: {} bytes", esp_alloc::HEAP.free());
    println!("=========================================");

    // 3. Build the composite dispatcher. Port registration is delegated to
    //    the active chip module — pin assignments and peripheral wiring are
    //    chip-specific, but the resulting `CompositeDispatcher` and the
    //    `ChipBoot` (host UART + optional wifi state) are not.
    let mut composite = CompositeDispatcher::new();

    let chip_boot = chip::active::register_all_ports(&mut composite, peripherals);

    println!(
        "[port] composite has {} ports, {} primitives total",
        composite.port_count(),
        composite.list_primitives().len()
    );
    println!();

    // 4. Wrap the composite in a leaf state (routine storage + dispatch).
    let mut leaf = LeafState::new(composite);

    // 5. Print the body's full self-model.
    print_self_model(&mut leaf);

    // 6. Self-test: brain transfers a cross-port routine and invokes it.
    //    The test pin is chip-specific (the only pin claimed by the gpio
    //    port differs per chip).
    run_brain_self_test(&mut leaf, chip::active::TEST_LED_PIN);

    // 7. Auto-connect to stored wifi credentials, if any.
    //
    // This MUST happen before run_dual_transport starts so that when
    // smoltcp's DhcpSocket polls for the first time, the radio is already
    // associated. Otherwise smoltcp gives up on DHCP (link down at poll
    // time) and never retries, leaving the chip forever without an IP
    // even after a successful wifi.configure later.
    //
    // Credentials are read from FlashKvStore (the same store used by
    // wifi.configure to persist them). If no credentials are stored, the
    // block is a no-op and the host can configure via wifi.configure
    // followed by a reset.
    #[cfg(feature = "wifi")]
    {
        let store = FlashKvStore::new();
        let ssid = store.get("wifi.ssid").ok().flatten();
        let password = store.get("wifi.password").ok().flatten();
        if let (Some(ssid), Some(password)) = (ssid, password) {
            println!("[wifi] auto-connecting to stored network '{}'", ssid);
            let resp = leaf.handle(TransportMessage::InvokeSkill {
                peer_id: "boot".to_string(),
                skill_id: "wifi.configure".to_string(),
                input: serde_json::json!({
                    "ssid": ssid,
                    "password": password,
                }),
            });
            if let TransportResponse::SkillResult { response } = &resp {
                if response.success {
                    println!("[wifi] auto-connect OK — radio associated, waiting for DHCP");
                } else {
                    println!(
                        "[wifi] auto-connect FAILED: {:?}",
                        response.failure_message
                    );
                }
            }
        } else {
            println!("[wifi] no stored credentials — use wifi.configure to set them");
        }
    }

    println!("=========================================");
    println!("Leaf transports active");
    println!("  Free heap before dispatch: {} bytes", esp_alloc::HEAP.free());
    println!("  UART0 (host wire frames): yes (115200 8N1)");
    #[cfg(feature = "wifi")]
    {
        if chip_boot.wifi_iface.is_some() && chip_boot.wifi_device.is_some() {
            println!("  TCP over WiFi:           yes (port 9100, when WiFi connected)");
        } else {
            println!("  TCP over WiFi:           disabled (esp-wifi unavailable)");
        }
    }
    #[cfg(not(feature = "wifi"))]
    println!("  TCP over WiFi:           disabled (built without --features wifi)");
    println!("Body alive, awaiting brain messages");
    println!("=========================================");

    #[cfg(feature = "wifi")]
    run_dual_transport(
        chip_boot.host_uart,
        chip_boot.wifi_iface,
        chip_boot.wifi_device,
        &mut leaf,
    );
    #[cfg(not(feature = "wifi"))]
    run_uart_transport(chip_boot.host_uart, &mut leaf);
}

// ---------------------------------------------------------------------------
// Dual transport loop: USB serial + TCP over WiFi
// ---------------------------------------------------------------------------
//
// Polls the USB serial transport AND (if available) the smoltcp TCP listener
// each iteration. Both feed the same LeafState. The brain can talk to the
// device over either path:
//
//   - USB serial: works immediately, used for initial WiFi configuration
//   - TCP/9100:   works once WiFi is connected, used for normal operation
//
// Currently the TCP listener accepts a single connection at a time. When the
// connection drops, the socket reverts to listen state for the next client.

const RX_BUFFER_CAP: usize = 4096;
#[cfg(feature = "wifi")]
const TCP_PORT: u16 = 9100;
#[cfg(feature = "wifi")]
const TCP_RX_BUF_SIZE: usize = 4096;
#[cfg(feature = "wifi")]
const TCP_TX_BUF_SIZE: usize = 4096;

/// UART-only dispatch loop. Used when the wifi feature is disabled, so the
/// firmware doesn't need esp-wifi or smoltcp.
#[cfg(not(feature = "wifi"))]
fn run_uart_transport<D: SkillDispatcher>(
    mut host_uart: Uart<'static, esp_hal::Blocking>,
    leaf: &mut LeafState<D>,
) -> ! {
    let mut uart_rx_buf: alloc::vec::Vec<u8> =
        alloc::vec::Vec::with_capacity(RX_BUFFER_CAP);
    let delay = Delay::new();
    let mut byte_buf = [0u8; 64];

    loop {
        match host_uart.read_buffered_bytes(&mut byte_buf) {
            Ok(0) => {}
            Ok(n) => {
                if uart_rx_buf.len() + n > RX_BUFFER_CAP {
                    uart_rx_buf.clear();
                }
                uart_rx_buf.extend_from_slice(&byte_buf[..n]);
            }
            Err(_) => {}
        }
        try_dispatch_frame(&mut uart_rx_buf, leaf, |bytes| {
            let _ = host_uart.write_bytes(bytes);
        });
        delay.delay_millis(1);
    }
}

/// Dual-transport dispatch loop. Used when the wifi feature is enabled.
/// Polls UART0 host frames AND (if DHCP succeeded) the smoltcp TCP listener
/// each iteration. Both feed the same LeafState. The brain can talk to the
/// device over either path interchangeably.
#[cfg(feature = "wifi")]
fn run_dual_transport<D: SkillDispatcher>(
    mut host_uart: Uart<'static, esp_hal::Blocking>,
    iface: Option<&'static mut Interface>,
    device: Option<&'static mut WifiDevice<'static, WifiStaDevice>>,
    leaf: &mut LeafState<D>,
) -> ! {
    let mut uart_rx_buf: alloc::vec::Vec<u8> =
        alloc::vec::Vec::with_capacity(RX_BUFFER_CAP);
    let delay = Delay::new();

    let mut tcp_state: Option<TcpListenerState> = match (iface, device) {
        (Some(iface), Some(device)) => Some(TcpListenerState::new(iface, device)),
        _ => None,
    };

    let mut tcp_rx_buf: alloc::vec::Vec<u8> =
        alloc::vec::Vec::with_capacity(RX_BUFFER_CAP);
    let mut byte_buf = [0u8; 64];

    loop {
        // ---- UART0 host path ----
        match host_uart.read_buffered_bytes(&mut byte_buf) {
            Ok(0) => {}
            Ok(n) => {
                if uart_rx_buf.len() + n > RX_BUFFER_CAP {
                    uart_rx_buf.clear();
                }
                uart_rx_buf.extend_from_slice(&byte_buf[..n]);
            }
            Err(_) => {}
        }
        try_dispatch_frame(&mut uart_rx_buf, leaf, |bytes| {
            let _ = host_uart.write_bytes(bytes);
        });

        // ---- TCP path ----
        if let Some(ref mut tcp) = tcp_state {
            tcp.poll(&mut tcp_rx_buf, leaf);
        }

        delay.delay_millis(1);
    }
}

/// Decode and dispatch any complete frame in `rx_buf`. Calls `write_cb` with
/// the encoded response bytes if a frame was processed.
fn try_dispatch_frame<D, F>(rx_buf: &mut alloc::vec::Vec<u8>, leaf: &mut LeafState<D>, mut write_cb: F)
where
    D: SkillDispatcher,
    F: FnMut(&[u8]),
{
    if rx_buf.len() < 4 {
        return;
    }
    match decode_frame(rx_buf, DEFAULT_MAX_FRAME) {
        Ok((msg, consumed)) => {
            let response = leaf.handle(msg);
            if let Ok(bytes) = encode_response(&response) {
                write_cb(&bytes);
            }
            rx_buf.drain(..consumed);
        }
        Err(FrameError::NeedMore) => {}
        Err(FrameError::TooLarge) => {
            rx_buf.clear();
            let err = TransportResponse::Error {
                details: "frame exceeds max size".to_string(),
            };
            if let Ok(bytes) = encode_response(&err) {
                write_cb(&bytes);
            }
        }
        Err(FrameError::Decode) => {
            let len =
                u32::from_be_bytes([rx_buf[0], rx_buf[1], rx_buf[2], rx_buf[3]]) as usize;
            let total = 4 + len;
            if rx_buf.len() >= total {
                rx_buf.drain(..total);
            }
            let err = TransportResponse::Error {
                details: "json decode failed inside frame".to_string(),
            };
            if let Ok(bytes) = encode_response(&err) {
                write_cb(&bytes);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TCP listener state — wraps smoltcp Interface + SocketSet + TcpSocket.
//
// Compiled only when the `wifi` cargo feature is enabled. The wifi feature
// is currently off by default because esp-wifi 0.12 transitively pulls
// esp-alloc 0.6 which panics on 0-byte allocations from background tasks
// in the dispatch loop. Until that's worked around, this whole subsystem
// stays cfg-gated.
// ---------------------------------------------------------------------------

#[cfg(feature = "wifi")]
struct TcpListenerState {
    iface: &'static mut Interface,
    device: &'static mut WifiDevice<'static, WifiStaDevice>,
    /// Box-leaked SocketSet so we can hold a 'static handle alongside the iface.
    /// The SocketSet internally borrows the leaked storage slice.
    sockets: &'static mut SocketSet<'static>,
    tcp_handle: smoltcp::iface::SocketHandle,
    dhcp_handle: smoltcp::iface::SocketHandle,
    /// Set once DHCP completes — used to display the assigned IP and to
    /// know whether the TCP listener should accept connections yet.
    assigned_addr: Option<Ipv4Address>,
    /// Iteration counter so we can kick the DhcpSocket periodically when
    /// it's stuck in Deconfigured state. Incremented every run_dual_transport
    /// iteration (~1 ms each); 5000 ≈ 5 seconds.
    poll_iters_since_reset: u32,
    /// mDNS responder that announces this leaf as `_soma._tcp.local`.
    /// Populated when DHCP assigns an address, cleared on lease loss.
    mdns: mdns::MdnsResponder,
}

#[cfg(feature = "wifi")]
impl TcpListenerState {
    fn new(
        iface: &'static mut Interface,
        device: &'static mut WifiDevice<'static, WifiStaDevice>,
    ) -> Self {
        // Allocate buffers and storage on the heap, leak to 'static.
        let rx_buf =
            Box::leak(alloc::vec![0u8; TCP_RX_BUF_SIZE].into_boxed_slice());
        let tx_buf =
            Box::leak(alloc::vec![0u8; TCP_TX_BUF_SIZE].into_boxed_slice());
        let socket = TcpSocket::new(
            TcpSocketBuffer::new(rx_buf),
            TcpSocketBuffer::new(tx_buf),
        );

        // SocketStorage<'_> isn't Clone, so build the array manually.
        // Slots: tcp + dhcp + udp (mDNS) + spare = 4
        let sockets_storage: &'static mut [SocketStorage<'static>] = Box::leak(
            alloc::vec::Vec::from([
                SocketStorage::EMPTY,
                SocketStorage::EMPTY,
                SocketStorage::EMPTY,
                SocketStorage::EMPTY,
            ])
            .into_boxed_slice(),
        );
        let sockets: &'static mut SocketSet<'static> =
            Box::leak(Box::new(SocketSet::new(&mut sockets_storage[..])));
        let tcp_handle = sockets.add(socket);

        // DHCP socket. Drives auto-configuration of the IP address from the
        // network. Without this we'd need a static IP, which is fine for
        // some setups but not for typical home WiFi.
        let dhcp_socket = DhcpSocket::new();
        let dhcp_handle = sockets.add(dhcp_socket);

        // mDNS responder. Uses the interface's already-assigned MAC
        // address as the unique identifier in the service instance name.
        // Joining the multicast group happens later inside this method
        // so smoltcp accepts 224.0.0.251 traffic.
        let mac_bytes = {
            let smoltcp::wire::HardwareAddress::Ethernet(eth) = iface.hardware_addr();
            let a = eth.as_bytes();
            [a[0], a[1], a[2], a[3], a[4], a[5]]
        };
        let mdns = mdns::MdnsResponder::new(sockets, mac_bytes, chip::active::NAME_LOWER);

        // Join the mDNS multicast group so incoming queries reach our
        // UdpSocket. smoltcp silently drops multicast traffic otherwise.
        let _ = iface.join_multicast_group(Ipv4Address::new(224, 0, 0, 251));

        Self {
            iface,
            device,
            sockets,
            tcp_handle,
            dhcp_handle,
            assigned_addr: None,
            poll_iters_since_reset: 0,
            mdns,
        }
    }

    fn now() -> smoltcp::time::Instant {
        smoltcp::time::Instant::from_micros(
            esp_hal::time::now().duration_since_epoch().to_micros() as i64,
        )
    }

    fn poll<D: SkillDispatcher>(
        &mut self,
        rx_buf: &mut alloc::vec::Vec<u8>,
        leaf: &mut LeafState<D>,
    ) {
        // Drive the smoltcp state machine.
        self.iface.poll(Self::now(), self.device, self.sockets);

        // Drive the mDNS responder. It drains any queries that landed
        // in the UdpSocket rx buffer since the last poll and sends
        // replies. The first time this runs after DHCP gives us an
        // address, it also sends a gratuitous announcement.
        self.mdns.poll(self.sockets);

        // DhcpSocket retry kick.
        //
        // esp-wifi 0.12 + smoltcp 0.12 have a link-state race: if the
        // DhcpSocket polls while the wifi link is still down, it emits
        // Deconfigured once and then waits the internal DHCP backoff
        // (can be minutes) before retrying. When the link later comes
        // up (after wifi.configure / auto-connect), DhcpSocket doesn't
        // notice and stays silent.
        //
        // Workaround: reset the DhcpSocket every ~3 seconds while we
        // don't have an address. DhcpSocket::reset restarts the
        // discovery state machine immediately. Once we get an address
        // (Configured), we stop resetting.
        self.poll_iters_since_reset = self.poll_iters_since_reset.wrapping_add(1);
        if self.assigned_addr.is_none() && self.poll_iters_since_reset >= 3000 {
            let dhcp = self.sockets.get_mut::<DhcpSocket>(self.dhcp_handle);
            dhcp.reset();
            println!("[net] DhcpSocket reset (no address yet)");
            self.poll_iters_since_reset = 0;
        }

        // Process DHCP events. On Configured we install the assigned IP
        // into the interface so the TCP socket can be reached.
        let dhcp_socket = self.sockets.get_mut::<DhcpSocket>(self.dhcp_handle);
        if let Some(event) = dhcp_socket.poll() {
            match event {
                DhcpEvent::Configured(config) => {
                    let ip = config.address.address();
                    self.assigned_addr = Some(ip);
                    println!("[net] DHCP assigned: {}", ip);
                    self.iface.update_ip_addrs(|addrs| {
                        addrs.clear();
                        let _ = addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(
                            ip,
                            config.address.prefix_len(),
                        )));
                    });
                    if let Some(router) = config.router {
                        self.iface
                            .routes_mut()
                            .add_default_ipv4_route(router)
                            .ok();
                    } else {
                        self.iface.routes_mut().remove_default_ipv4_route();
                    }
                    // Tell the mDNS responder our new address so it can
                    // announce itself and answer A queries for our
                    // hostname.
                    self.mdns.set_ipv4(ip);
                    // Publish the IP to the lock-free global so
                    // RealWifiOps::status can see it without needing a
                    // back-channel from TcpListenerState.
                    set_assigned_ipv4(ip);
                }
                DhcpEvent::Deconfigured => {
                    println!("[net] DHCP lease lost");
                    self.assigned_addr = None;
                    self.iface.update_ip_addrs(|addrs| addrs.clear());
                    self.iface.routes_mut().remove_default_ipv4_route();
                    self.mdns.clear_ipv4();
                    clear_assigned_ipv4();
                }
            }
        }

        // Don't try to listen until DHCP gave us an address.
        if self.assigned_addr.is_none() {
            return;
        }

        let socket = self.sockets.get_mut::<TcpSocket>(self.tcp_handle);

        // Socket state machine:
        //   Listen       — waiting for incoming SYN, ready to accept
        //   SynReceived  — handshake in progress
        //   Established  — connected, exchanging data
        //   FinWait*, CloseWait, Closing, LastAck, TimeWait — closing
        //   Closed       — inactive, ready to listen again
        //
        // Problem: soma-next's TcpRemoteExecutor opens a fresh TCP
        // connection per invoke_remote_skill call and closes it after
        // one request/response. smoltcp's TCP socket enters TIME_WAIT
        // on close and stays there for the MSL window (default ~60 s).
        // During TIME_WAIT, `is_open()` returns TRUE, so the previous
        // version of this code never re-listened — soma-next's second
        // invoke hit Connection refused.
        //
        // Fix: any state that isn't Listen AND isn't an active connection
        // (SynReceived / Established) is a transient closing state. Abort
        // the socket immediately to skip TIME_WAIT and call listen()
        // again so the next SYN is accepted.
        use smoltcp::socket::tcp::State as TcpState;
        let state = socket.state();
        let needs_reset = !matches!(
            state,
            TcpState::Listen | TcpState::SynReceived | TcpState::Established
        );
        if needs_reset {
            // abort() sends RST and returns the socket to Closed
            // immediately, bypassing TIME_WAIT. listen() then parks the
            // socket back in Listen state for the next connection.
            socket.abort();
            let _ = socket.listen(TCP_PORT);
            // Drop any stale rx bytes from the previous session.
            rx_buf.clear();
            return;
        }

        // Drain any received bytes into the rx buffer.
        if socket.can_recv() {
            let mut tmp = [0u8; 256];
            if let Ok(n) = socket.recv_slice(&mut tmp) {
                if rx_buf.len() + n > RX_BUFFER_CAP {
                    rx_buf.clear();
                }
                rx_buf.extend_from_slice(&tmp[..n]);
            }
        }

        // Try to decode a frame and dispatch.
        if rx_buf.len() >= 4 {
            match decode_frame(rx_buf, DEFAULT_MAX_FRAME) {
                Ok((msg, consumed)) => {
                    let response = leaf.handle(msg);
                    if let Ok(bytes) = encode_response(&response) {
                        if socket.can_send() {
                            let _ = socket.send_slice(&bytes);
                        }
                    }
                    rx_buf.drain(..consumed);
                }
                Err(FrameError::NeedMore) => {}
                Err(FrameError::TooLarge) | Err(FrameError::Decode) => {
                    rx_buf.clear();
                    socket.close();
                }
            }
        }
    }
}


// ---------------------------------------------------------------------------
// Proprioception
// ---------------------------------------------------------------------------

fn print_self_model<D: SkillDispatcher>(leaf: &mut LeafState<D>) {
    let resp = leaf.handle(TransportMessage::ListCapabilities);
    if let TransportResponse::Capabilities {
        primitives,
        routines,
    } = resp
    {
        println!("=========================================");
        println!("Body self-model");
        println!("  primitives: {}", primitives.len());
        println!("  routines:   {}", routines.len());
        println!("=========================================");
        for p in &primitives {
            let effect = match p.effect {
                soma_esp32_leaf::Effect::ReadOnly => "RO",
                soma_esp32_leaf::Effect::StateMutation => "SM",
                soma_esp32_leaf::Effect::ExternalEffect => "EX",
            };
            println!("  [{}] {}", effect, p.skill_id);
        }
        println!();
    }
}

// ---------------------------------------------------------------------------
// Self-test: cross-port routine via TransferRoutine
// ---------------------------------------------------------------------------

fn run_brain_self_test<D: SkillDispatcher>(leaf: &mut LeafState<D>, test_pin: u32) {
    println!("=========================================");
    println!("Self-test: brain composes a cross-port routine");
    println!("=========================================");

    // A cross-port routine: status -> blink LED -> wait -> blink again.
    // Uses primitives from gpio, delay, and (if available) thermistor. The
    // gpio pin is supplied by the active chip module so this routine works
    // unchanged on every supported chip.
    let cross_port_routine = Routine {
        routine_id: "demo_pulse".to_string(),
        description: "GPIO toggle, delay, gpio toggle — demonstrates routine walking"
            .to_string(),
        steps: alloc::vec![
            #[cfg(feature = "thermistor")]
            RoutineStep {
                skill_id: "thermistor.read_temp".to_string(),
                input: json!({ "channel": 0 }),
            },
            RoutineStep {
                skill_id: "gpio.toggle".to_string(),
                input: json!({ "pin": test_pin }),
            },
            RoutineStep {
                skill_id: "delay.ms".to_string(),
                input: json!({ "ms": 200 }),
            },
            RoutineStep {
                skill_id: "gpio.toggle".to_string(),
                input: json!({ "pin": test_pin }),
            },
        ],
    };

    println!(
        "[1] Brain transfers routine '{}' with {} steps",
        cross_port_routine.routine_id,
        cross_port_routine.steps.len()
    );
    let resp = leaf.handle(TransportMessage::TransferRoutine {
        routine: cross_port_routine,
    });
    if let TransportResponse::RoutineStored {
        routine_id,
        step_count,
    } = resp
    {
        println!(
            "    leaf acknowledged: stored '{}' ({} steps)",
            routine_id, step_count
        );
    }
    println!();

    println!("[2] Brain invokes 'demo_pulse'");
    let resp = leaf.handle(TransportMessage::InvokeSkill {
        peer_id: "self_test".to_string(),
        skill_id: "demo_pulse".to_string(),
        input: json!({}),
    });
    if let TransportResponse::SkillResult { response } = &resp {
        println!(
            "    response: success={}, steps_executed={}",
            response.success, response.steps_executed
        );
    }
    println!();

    println!("--- self-test complete ---");
    println!();
}
