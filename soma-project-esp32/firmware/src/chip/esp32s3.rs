// chip::esp32s3 — pin assignments and peripheral wiring for ESP32-S3.
//
// Tested against the Sunton ESP32-S3-1732S019 dev board (16 MB flash,
// 8 MB PSRAM, on-board CH340 USB-to-UART bridge). The on-board ST7789
// LCD sits on GPIO 1, 4-7, 10-14, 38-41 — this firmware does NOT touch
// those pins by default; the LCD stays in reset.
//
// ARCHITECTURE — runtime-configurable pins:
//
// This module no longer hardcodes pin numbers for each peripheral.
// Instead, at boot:
//
//   1. read pin assignments from FlashKvStore under `pins.<peripheral>`
//      keys (e.g. `pins.i2c0.sda`). Values are ASCII u8 decimal strings.
//
//   2. fall back to per-chip defaults (the constants at the top of this
//      file) if a key is missing.
//
//   3. use `esp_hal::gpio::AnyPin::steal(n)` to dispatch a runtime u8
//      to a typed pin handle for peripherals that accept `AnyPin`.
//
//   4. for ADC (which requires a statically-known `GpioPin<N>`) use a
//      match over the valid ADC1 pins and invoke typed construction
//      on each arm. Only one arm runs per boot, so `peripherals.ADC1`
//      is moved exactly once.
//
// Critical pin safety rules for ESP32-S3:
//   - GPIO 26-32 are reserved for the internal QSPI flash and PSRAM on
//     the common 16 MB / 8 MB PSRAM WROOM modules. Touching them
//     corrupts flash access. VALID_GPIOS excludes them.
//   - GPIO 0, 45, 46 are strapping pins. Usable after boot but can
//     prevent first-stage boot if in an unexpected state at reset.
//   - On the Sunton 1732S019 board, GPIO 1/4-7/10-14/38-41 are owned
//     by the on-board ST7789 LCD. If you are not using the LCD you
//     can remap these via `board.configure_pin`.

use alloc::boxed::Box;
#[cfg(feature = "board")]
use alloc::string::String;
#[cfg(feature = "board")]
use alloc::vec;
#[cfg(feature = "board")]
use alloc::vec::Vec;

use esp_hal::{
    clock::CpuClock,
    gpio::{AnyPin, Level, Output},
    peripherals::Peripherals,
    uart::{Config as HostUartConfig, Uart},
};
#[cfg(feature = "board")]
use esp_hal::efuse::Efuse;
use esp_println::println;

use soma_esp32_leaf::CompositeDispatcher;
use soma_esp32_port_storage::KvStore;

use crate::ChipBoot;
#[cfg(feature = "wifi")]
use esp_wifi::wifi::WifiController;

/// Friendly chip name printed in the boot banner.
pub const NAME: &str = "ESP32-S3";

/// Lowercase chip name used inside mDNS hostnames and other wire-format
/// identifiers. Must be ASCII, DNS-safe (a-z, 0-9, '-').
#[cfg(feature = "wifi")]
pub const NAME_LOWER: &str = "esp32s3";

/// Default pin assignments — used when no `pins.*` key is stored in
/// FlashKvStore. Tuned for the Sunton ESP32-S3-1732S019 board.
pub const DEFAULT_GPIO_TEST: u8 = 15;
pub const DEFAULT_I2C_SDA: u8 = 8;
pub const DEFAULT_I2C_SCL: u8 = 9;
pub const DEFAULT_SPI_SCK: u8 = 35;
pub const DEFAULT_SPI_MOSI: u8 = 36;
pub const DEFAULT_ADC_PIN: u8 = 2;
pub const DEFAULT_PWM_PIN: u8 = 16;
pub const DEFAULT_UART1_TX: u8 = 17;
pub const DEFAULT_UART1_RX: u8 = 18;

/// GPIO pin number reserved for gpio.write/read/toggle and used by the
/// boot self-test routine. Read dynamically at boot but exposed as a
/// constant for the self-test which is compile-time.
pub const TEST_LED_PIN: u32 = DEFAULT_GPIO_TEST as u32;

/// Valid GPIO numbers for the ESP32-S3. Excludes GPIO 26-32 which are
/// reserved for the internal QSPI flash and PSRAM on WROOM modules.
/// Strapping pins (0, 45, 46) are allowed — they work after boot, user
/// is responsible for not breaking the reset sequence.
const VALID_GPIOS: &[u8] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 33, 34, 35, 36,
    37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
];

fn is_valid_gpio(n: u8) -> bool {
    VALID_GPIOS.contains(&n)
}

/// Safely steal a typed AnyPin for the given GPIO number. Falls back
/// to `fallback` if the number is in the QSPI-flash/PSRAM-reserved
/// range or otherwise invalid.
fn any_output_pin(n: u8, fallback: u8) -> AnyPin {
    if !is_valid_gpio(n) {
        println!(
            "[chip] pin GPIO{} is not a valid user pin on ESP32-S3 (reserved for flash/PSRAM?), falling back to GPIO{}",
            n, fallback
        );
        return unsafe { AnyPin::steal(fallback) };
    }
    unsafe { AnyPin::steal(n) }
}

/// Same as `any_output_pin` but for inputs. Kept separate so the
/// warning text is accurate if future pin categories (input-only, etc.)
/// need to be distinguished.
fn any_input_pin(n: u8, fallback: u8) -> AnyPin {
    if !is_valid_gpio(n) {
        println!(
            "[chip] pin GPIO{} is not valid on ESP32-S3, falling back to GPIO{}",
            n, fallback
        );
        return unsafe { AnyPin::steal(fallback) };
    }
    unsafe { AnyPin::steal(n) }
}

/// Resolved runtime pin configuration for this chip. Populated at
/// boot from FlashKvStore, with defaults applied for any missing key.
pub struct PinConfig {
    pub gpio_test: u8,
    pub i2c0_sda: u8,
    pub i2c0_scl: u8,
    pub spi3_sck: u8,
    pub spi3_mosi: u8,
    pub adc_pin: u8,
    pub pwm_pin: u8,
    pub uart1_tx: u8,
    pub uart1_rx: u8,
}

impl PinConfig {
    /// Read every pin key from FlashKvStore, fall back to defaults.
    pub fn load(store: &crate::FlashKvStore) -> Self {
        fn parse(store: &crate::FlashKvStore, key: &str, default: u8) -> u8 {
            store
                .get(key)
                .ok()
                .flatten()
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(default)
        }
        Self {
            gpio_test: parse(store, "pins.gpio.test", DEFAULT_GPIO_TEST),
            i2c0_sda: parse(store, "pins.i2c0.sda", DEFAULT_I2C_SDA),
            i2c0_scl: parse(store, "pins.i2c0.scl", DEFAULT_I2C_SCL),
            spi3_sck: parse(store, "pins.spi3.sck", DEFAULT_SPI_SCK),
            spi3_mosi: parse(store, "pins.spi3.mosi", DEFAULT_SPI_MOSI),
            adc_pin: parse(store, "pins.adc.pin", DEFAULT_ADC_PIN),
            pwm_pin: parse(store, "pins.pwm.pin", DEFAULT_PWM_PIN),
            uart1_tx: parse(store, "pins.uart1.tx", DEFAULT_UART1_TX),
            uart1_rx: parse(store, "pins.uart1.rx", DEFAULT_UART1_RX),
        }
    }

    /// Return the list of (key, gpio) tuples for `board.pin_map`.
    #[cfg(feature = "board")]
    pub fn to_list(&self) -> Vec<(&'static str, u8)> {
        vec![
            ("pins.gpio.test", self.gpio_test),
            ("pins.i2c0.sda", self.i2c0_sda),
            ("pins.i2c0.scl", self.i2c0_scl),
            ("pins.spi3.sck", self.spi3_sck),
            ("pins.spi3.mosi", self.spi3_mosi),
            ("pins.adc.pin", self.adc_pin),
            ("pins.pwm.pin", self.pwm_pin),
            ("pins.uart1.tx", self.uart1_tx),
            ("pins.uart1.rx", self.uart1_rx),
        ]
    }
}

/// Initialize peripherals and clock for the ESP32-S3.
pub fn init_peripherals() -> Peripherals {
    esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    })
}

/// Read the test LED pin from flash (dynamic) so the boot self-test
/// uses the currently-configured pin.
#[allow(dead_code)]
pub fn current_test_led_pin() -> u8 {
    PinConfig::load(&crate::FlashKvStore::new()).gpio_test
}

/// Build & register every hardware port the firmware was built with into
/// the composite dispatcher, and return a `ChipBoot` containing the UART0
/// host transport handle plus optional wifi state (only when the wifi
/// cargo feature is enabled).
///
/// Consumes `peripherals` because the individual fields are moved into the
/// port closures and the wifi subsystem.
pub fn register_all_ports(
    composite: &mut CompositeDispatcher,
    peripherals: Peripherals,
) -> ChipBoot {
    // ----- wifi subsystem (only when feature = "wifi") -----
    //
    // MUST run before any other peripheral access so the wifi-related
    // fields (TIMG0, RNG, RADIO_CLK, WIFI) are moved out before the rest
    // of the function consumes GPIO/UART/etc.
    #[cfg(feature = "wifi")]
    let (wifi_iface, wifi_device, mut wifi_controller) = crate::init_wifi_subsystem(
        peripherals.TIMG0,
        peripherals.RNG,
        peripherals.RADIO_CLK,
        peripherals.WIFI,
    );

    // Load runtime pin configuration from FlashKvStore. Defaults used
    // for any missing keys.
    let pin_cfg = PinConfig::load(&crate::FlashKvStore::new());
    println!(
        "[chip] pin config loaded: i2c0 sda={} scl={}, spi3 sck={} mosi={}, adc={}, pwm={}, uart1 tx={} rx={}, gpio test={}",
        pin_cfg.i2c0_sda,
        pin_cfg.i2c0_scl,
        pin_cfg.spi3_sck,
        pin_cfg.spi3_mosi,
        pin_cfg.adc_pin,
        pin_cfg.pwm_pin,
        pin_cfg.uart1_tx,
        pin_cfg.uart1_rx,
        pin_cfg.gpio_test,
    );

    // ----- wifi port -----
    //
    // Even if the wifi cargo feature is enabled, the port only goes "live"
    // if esp-wifi initialization succeeded. The wifi port type-erases its
    // backend (`Box<dyn WifiOps>`) so the controller's chip-specific
    // generics don't leak into the port crate.
    #[cfg(feature = "wifi")]
    {
        let controller_for_port: Option<&'static mut WifiController<'static>> =
            wifi_controller.take();
        if let Some(controller_static) = controller_for_port {
            let real_ops = crate::RealWifiOps {
                controller: controller_static,
                store: crate::FlashKvStore::new(),
            };
            let wifi_port = soma_esp32_port_wifi::WifiPort::new(Box::new(real_ops));
            composite.register(Box::new(wifi_port));
            println!("[port] registered: wifi (RealWifiOps via esp-wifi station mode)");
        } else {
            let stub_ops = crate::StorageOnlyWifiOps {
                store: crate::FlashKvStore::new(),
            };
            let wifi_port = soma_esp32_port_wifi::WifiPort::new(Box::new(stub_ops));
            composite.register(Box::new(wifi_port));
            println!(
                "[port] registered: wifi (StorageOnlyWifiOps fallback — esp-wifi unavailable)"
            );
        }
    }

    // ----- gpio port -----
    #[cfg(feature = "gpio")]
    {
        let mut gpio_port = soma_esp32_port_gpio::GpioPort::new();
        let test_output = Output::new(
            any_output_pin(pin_cfg.gpio_test, DEFAULT_GPIO_TEST),
            Level::Low,
        );
        gpio_port.claim_output_pin(pin_cfg.gpio_test as u32, test_output);
        composite.register(Box::new(gpio_port));
        println!(
            "[port] registered: gpio (3 primitives, GPIO{} claimed)",
            pin_cfg.gpio_test
        );
    }

    // ----- delay port -----
    #[cfg(feature = "delay")]
    {
        let delay_port = soma_esp32_port_delay::DelayPort::new();
        composite.register(Box::new(delay_port));
        println!("[port] registered: delay (2 primitives)");
    }

    // ----- uart port (UART1) -----
    //
    // UART0 is reserved for host transport. UART1 pins come from the
    // runtime config.
    #[cfg(feature = "uart")]
    {
        use esp_hal::uart::{Config as UartConfig, Uart};
        let uart_config = UartConfig::default();
        match Uart::new(peripherals.UART1, uart_config) {
            Ok(uart) => {
                let uart = uart
                    .with_tx(any_output_pin(pin_cfg.uart1_tx, DEFAULT_UART1_TX))
                    .with_rx(any_input_pin(pin_cfg.uart1_rx, DEFAULT_UART1_RX));
                let uart_port = soma_esp32_port_uart::UartPort::new(uart);
                composite.register(Box::new(uart_port));
                println!(
                    "[port] registered: uart (UART1 on GPIO{}/GPIO{})",
                    pin_cfg.uart1_tx, pin_cfg.uart1_rx
                );
            }
            Err(e) => println!("[port] uart init failed: {:?}", e),
        }
    }

    // ----- i2c + display ports (shared I2C0 bus) -----
    //
    // Identical pattern to chip/esp32.rs: one leaked `&'static RefCell`
    // owns the bus, and each consumer (i2c port, display port) takes
    // its own `RefCellDevice` handle. See the esp32 module for the
    // full rationale.
    #[cfg(any(feature = "i2c", feature = "display"))]
    {
        use esp_hal::i2c::master::{Config as I2cConfig, I2c};
        match I2c::new(peripherals.I2C0, I2cConfig::default()) {
            Ok(i2c) => {
                let i2c = i2c
                    .with_sda(any_output_pin(pin_cfg.i2c0_sda, DEFAULT_I2C_SDA))
                    .with_scl(any_output_pin(pin_cfg.i2c0_scl, DEFAULT_I2C_SCL));

                #[cfg(feature = "display")]
                {
                    register_i2c_and_display(
                        composite,
                        i2c,
                        pin_cfg.i2c0_sda,
                        pin_cfg.i2c0_scl,
                    );
                }

                #[cfg(all(feature = "i2c", not(feature = "display")))]
                {
                    let i2c_port = soma_esp32_port_i2c::I2cPort::new(i2c);
                    composite.register(Box::new(i2c_port));
                    println!(
                        "[port] registered: i2c (I2C0 on GPIO{}/GPIO{})",
                        pin_cfg.i2c0_sda, pin_cfg.i2c0_scl
                    );
                }
            }
            Err(e) => println!("[port] i2c/display init failed: {:?}", e),
        }
    }

    // ----- spi port (SPI3) -----
    //
    // SPI3 is used here because SPI2 is the natural choice for the on-board
    // ST7789 display on the 1732S019. Keeping the user-facing SPI port off
    // SPI2 means a future display port can take SPI2 without conflicts.
    #[cfg(feature = "spi")]
    {
        use esp_hal::spi::master::{Config as SpiConfig, Spi};
        match Spi::new(peripherals.SPI3, SpiConfig::default()) {
            Ok(spi) => {
                let spi = spi
                    .with_sck(any_output_pin(pin_cfg.spi3_sck, DEFAULT_SPI_SCK))
                    .with_mosi(any_output_pin(pin_cfg.spi3_mosi, DEFAULT_SPI_MOSI));
                let spi_port = soma_esp32_port_spi::SpiPort::new(spi);
                composite.register(Box::new(spi_port));
                println!(
                    "[port] registered: spi (SPI3 on GPIO{}/GPIO{})",
                    pin_cfg.spi3_sck, pin_cfg.spi3_mosi
                );
            }
            Err(e) => println!("[port] spi init failed: {:?}", e),
        }
    }

    // ----- adc port -----
    //
    // The esp-hal ADC API requires a concrete `GpioPin<N>` — it won't
    // accept `AnyPin` because the `AdcChannel` trait impls only exist
    // for the statically-known pin types. That rules out a single
    // runtime-dispatched construction path.
    //
    // Workaround: enumerate every ADC1-capable pin in a match. Each
    // arm constructs its own typed ADC instance and wraps it in the
    // same `AdcReadFn` closure. At runtime only one arm runs, so
    // `peripherals.ADC1` is moved exactly once.
    //
    // ADC1 mapping on ESP32-S3:
    //   GPIO1 = CH0, GPIO2 = CH1, GPIO3 = CH2, GPIO4 = CH3,
    //   GPIO5 = CH4, GPIO6 = CH5, GPIO7 = CH6, GPIO8 = CH7,
    //   GPIO9 = CH8, GPIO10 = CH9
    #[cfg(feature = "adc")]
    {
        let resolved_adc_pin = if adc_channel_for_pin(pin_cfg.adc_pin).is_some() {
            pin_cfg.adc_pin
        } else {
            println!(
                "[chip] adc_pin GPIO{} has no ADC1 channel on ESP32-S3, falling back to GPIO{}",
                pin_cfg.adc_pin, DEFAULT_ADC_PIN
            );
            DEFAULT_ADC_PIN
        };

        macro_rules! adc_arm {
            ($pin_n:literal, $channel:literal) => {{
                use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
                let mut adc_config = AdcConfig::new();
                let mut adc_pin = adc_config.enable_pin(
                    unsafe { esp_hal::gpio::GpioPin::<$pin_n>::steal() },
                    Attenuation::_11dB,
                );
                let mut adc = Adc::new(peripherals.ADC1, adc_config);
                let read_fn: soma_esp32_port_adc::AdcReadFn = Box::new(move || {
                    nb::block!(adc.read_oneshot(&mut adc_pin))
                        .map_err(|_| soma_esp32_port_adc::AdcError::HardwareError)
                });
                soma_esp32_port_adc::AdcPort::new($channel as u32, read_fn)
            }};
        }

        let (adc_port, channel) = match resolved_adc_pin {
            1 => (adc_arm!(1, 0u8), 0u8),
            2 => (adc_arm!(2, 1u8), 1u8),
            3 => (adc_arm!(3, 2u8), 2u8),
            4 => (adc_arm!(4, 3u8), 3u8),
            5 => (adc_arm!(5, 4u8), 4u8),
            6 => (adc_arm!(6, 5u8), 5u8),
            7 => (adc_arm!(7, 6u8), 6u8),
            8 => (adc_arm!(8, 7u8), 7u8),
            9 => (adc_arm!(9, 8u8), 8u8),
            10 => (adc_arm!(10, 9u8), 9u8),
            _ => unreachable!("adc pin validated above"),
        };
        composite.register(Box::new(adc_port));
        println!(
            "[port] registered: adc (ADC1 channel {} on GPIO{})",
            channel, resolved_adc_pin
        );
    }

    // ----- pwm port -----
    #[cfg(feature = "pwm")]
    {
        use esp_hal::ledc::{
            channel::{self, ChannelIFace},
            timer::{self, TimerIFace},
            LSGlobalClkSource, Ledc, LowSpeed,
        };
        use fugit::RateExtU32;

        let ledc_static: &'static mut Ledc<'static> =
            Box::leak(Box::new(Ledc::new(peripherals.LEDC)));
        ledc_static.set_global_slow_clock(LSGlobalClkSource::APBClk);

        let timer_static: &'static mut esp_hal::ledc::timer::Timer<'static, LowSpeed> =
            Box::leak(Box::new(ledc_static.timer::<LowSpeed>(timer::Number::Timer0)));
        timer_static
            .configure(timer::config::Config {
                duty: timer::config::Duty::Duty8Bit,
                clock_source: timer::LSClockSource::APBClk,
                frequency: 1u32.kHz(),
            })
            .unwrap();

        let mut channel0 = ledc_static.channel(
            channel::Number::Channel0,
            any_output_pin(pin_cfg.pwm_pin, DEFAULT_PWM_PIN),
        );
        channel0
            .configure(channel::config::Config {
                timer: timer_static,
                duty_pct: 0,
                pin_config: channel::config::PinConfig::PushPull,
            })
            .unwrap();

        #[allow(unused_mut)]
        let mut channel_owned: esp_hal::ledc::channel::Channel<'static, LowSpeed> = channel0;
        let set_duty_fn: soma_esp32_port_pwm::PwmSetDutyFn = Box::new(move |duty: u8| {
            channel_owned
                .set_duty(duty)
                .map_err(|_| soma_esp32_port_pwm::PwmError::HardwareError)
        });

        let pwm_port = soma_esp32_port_pwm::PwmPort::new(0, 1000, set_duty_fn);
        composite.register(Box::new(pwm_port));
        println!(
            "[port] registered: pwm (LEDC channel 0 on GPIO{}, 1kHz)",
            pin_cfg.pwm_pin
        );
    }

    // ----- storage port (real NVS-backed via esp-storage) -----
    #[cfg(feature = "storage")]
    {
        let store = crate::FlashKvStore::new();
        let storage_port = soma_esp32_port_storage::StoragePort::new(Box::new(store));
        composite.register(Box::new(storage_port));
        println!(
            "[port] registered: storage (FlashKvStore on SPI flash sector {:#x})",
            crate::FlashKvStore::SECTOR_OFFSET
        );
    }

    // ----- thermistor port (example sensor) -----
    #[cfg(feature = "thermistor")]
    {
        let thermistor = soma_esp32_port_thermistor::ThermistorPort::new();
        composite.register(Box::new(thermistor));
        println!("[port] registered: thermistor (simulated)");
    }

    // ----- board port (diagnostic + config) -----
    //
    // Injects closures that the board port uses to run its primitives.
    // probe_i2c_buses uses unsafe peripheral stealing — after the probe
    // runs, the I²C peripheral is in an unknown state and the i2c
    // port's bound instance is invalidated. Users should call
    // board.reboot after probing to restore a clean state.
    #[cfg(feature = "board")]
    {
        let chip_info_fn: soma_esp32_port_board::ChipInfoFn = Box::new(move || {
            soma_esp32_port_board::ChipInfo {
                chip: NAME,
                mac: Efuse::read_base_mac_address(),
                free_heap: esp_alloc::HEAP.free() as u32,
                uptime_ms: esp_hal::time::now()
                    .duration_since_epoch()
                    .to_millis(),
                firmware_version: env!("CARGO_PKG_VERSION"),
            }
        });

        let pin_map_fn: soma_esp32_port_board::PinMapFn = Box::new(move || {
            PinConfig::load(&crate::FlashKvStore::new()).to_list()
        });

        let probe_i2c_fn: soma_esp32_port_board::ProbeI2cFn =
            Box::new(move |candidates: &[(u8, u8)]| probe_i2c_impl(candidates));

        let reboot_fn: soma_esp32_port_board::RebootFn = Box::new(move || {
            println!("[board] rebooting on request");
            esp_hal::reset::software_reset();
        });

        let configure_fn: soma_esp32_port_board::ConfigureFn =
            Box::new(move |key: &str, value: &str| -> Result<(), String> {
                let mut store = crate::FlashKvStore::new();
                store
                    .set(key, value)
                    .map_err(|e| alloc::format!("{:?}", e))
            });

        let board_port = soma_esp32_port_board::BoardPort::new(
            chip_info_fn,
            pin_map_fn,
            probe_i2c_fn,
            reboot_fn,
            configure_fn,
        );
        composite.register(Box::new(board_port));
        println!("[port] registered: board (5 primitives: chip_info/pin_map/configure_pin/probe_i2c_buses/reboot)");
    }

    // ----- UART0 host transport — return the uart handle to caller -----
    let host_uart = Uart::new(peripherals.UART0, HostUartConfig::default())
        .expect("UART0 init for host transport");

    ChipBoot {
        host_uart,
        #[cfg(feature = "wifi")]
        wifi_iface,
        #[cfg(feature = "wifi")]
        wifi_device,
        #[cfg(feature = "wifi")]
        wifi_controller,
    }
}

/// Map an ADC1 GPIO pin number to its channel index on ESP32-S3.
/// Returns None for pins that don't have an ADC1 mapping.
#[cfg(feature = "adc")]
fn adc_channel_for_pin(gpio: u8) -> Option<u8> {
    match gpio {
        1 => Some(0),
        2 => Some(1),
        3 => Some(2),
        4 => Some(3),
        5 => Some(4),
        6 => Some(5),
        7 => Some(6),
        8 => Some(7),
        9 => Some(8),
        10 => Some(9),
        _ => None,
    }
}

/// Probe each candidate (sda, scl) pair by unsafely stealing I2C0 and
/// the pin peripherals, initializing I²C, running a scan, and reporting
/// what responded.
///
/// WARNING: this invalidates any prior I²C state. After running a
/// probe, the caller should call `board.reboot` so the firmware's
/// primary I²C port comes back up cleanly on the configured pins.
#[cfg(feature = "board")]
fn probe_i2c_impl(candidates: &[(u8, u8)]) -> Vec<soma_esp32_port_board::ProbeResult> {
    use esp_hal::i2c::master::{Config as I2cConfig, I2c};

    let mut results = Vec::new();
    for &(sda, scl) in candidates {
        if !is_valid_gpio(sda) || !is_valid_gpio(scl) {
            results.push(soma_esp32_port_board::ProbeResult {
                sda,
                scl,
                addresses: Vec::new(),
                error: Some(alloc::format!("invalid GPIO: sda={}, scl={}", sda, scl)),
            });
            continue;
        }

        // Steal the I2C0 peripheral and the requested pins.
        //
        // SAFETY: the caller has been warned that probing invalidates
        // the current I²C state. After the probe the peripheral is
        // dropped, then stolen again on the next candidate.
        let i2c0 = unsafe { esp_hal::peripherals::I2C0::steal() };
        let sda_pin = unsafe { AnyPin::steal(sda) };
        let scl_pin = unsafe { AnyPin::steal(scl) };

        match I2c::new(i2c0, I2cConfig::default()) {
            Ok(i2c) => {
                let mut i2c = i2c.with_sda(sda_pin).with_scl(scl_pin);

                // Scan 7-bit addresses 0x08..0x78 (reserved ranges
                // excluded per I²C spec).
                let mut found = Vec::new();
                for addr in 0x08u8..=0x77u8 {
                    // Write 0 bytes — the probe is an address-only
                    // transaction. Devices that ACK are present.
                    let buf = [0u8; 1];
                    if i2c.write(addr, &buf[..0]).is_ok() {
                        found.push(addr);
                    }
                }

                results.push(soma_esp32_port_board::ProbeResult {
                    sda,
                    scl,
                    addresses: found,
                    error: None,
                });
                // i2c dropped here — the peripheral returns to the
                // hardware. The next iteration steals it fresh.
            }
            Err(e) => {
                results.push(soma_esp32_port_board::ProbeResult {
                    sda,
                    scl,
                    addresses: Vec::new(),
                    error: Some(alloc::format!("I2C init failed: {:?}", e)),
                });
            }
        }
    }
    results
}

/// Shared-bus path for the i2c + display ports on ESP32-S3.
///
/// Identical to the esp32 module's helper. Kept local to each chip
/// module so adding a new chip only means dropping one file.
#[cfg(feature = "display")]
fn register_i2c_and_display(
    composite: &mut CompositeDispatcher,
    i2c: esp_hal::i2c::master::I2c<'static, esp_hal::Blocking>,
    sda_gpio: u8,
    scl_gpio: u8,
) {
    use core::cell::RefCell;
    use embedded_graphics::{
        mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
        pixelcolor::BinaryColor,
        prelude::*,
        primitives::{PrimitiveStyleBuilder, Rectangle},
        text::{Baseline, Text},
    };
    use embedded_hal_bus::i2c::RefCellDevice;
    use ssd1306::{
        mode::DisplayConfig, prelude::*, I2CDisplayInterface, Ssd1306,
    };

    const PANEL_I2C_ADDR: u8 = 0x3C;
    const PANEL_WIDTH: u16 = 128;
    const PANEL_HEIGHT: u16 = 64;
    const FONT_WIDTH_PX: u16 = 6;
    const FONT_HEIGHT_PX: u16 = 10;

    let bus_static: &'static RefCell<
        esp_hal::i2c::master::I2c<'static, esp_hal::Blocking>,
    > = Box::leak(Box::new(RefCell::new(i2c)));

    #[cfg(feature = "i2c")]
    {
        let i2c_device = RefCellDevice::new(bus_static);
        let i2c_port = soma_esp32_port_i2c::I2cPort::new(i2c_device);
        composite.register(Box::new(i2c_port));
        println!(
            "[port] registered: i2c (I2C0 shared bus on GPIO{}/GPIO{})",
            sda_gpio, scl_gpio
        );
    }

    let display_device = RefCellDevice::new(bus_static);
    let interface = I2CDisplayInterface::new(display_device);
    let mut ssd = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    if let Err(e) = ssd.init() {
        println!("[port] display init failed: {:?}", e);
        return;
    }

    let disp_static: &'static RefCell<_> = Box::leak(Box::new(RefCell::new(ssd)));

    let info_fn: soma_esp32_port_display::InfoFn =
        Box::new(move || soma_esp32_port_display::DisplayInfo {
            width: PANEL_WIDTH,
            height: PANEL_HEIGHT,
            driver: "ssd1306",
            i2c_addr: PANEL_I2C_ADDR,
        });

    let clear_fn: soma_esp32_port_display::ClearFn = Box::new(move || {
        let mut d = disp_static.borrow_mut();
        d.clear_buffer();
        d.flush().map_err(|e| alloc::format!("display clear: {:?}", e))
    });

    let draw_text_line_fn: soma_esp32_port_display::DrawTextLineFn =
        Box::new(move |text: &str, line: u8, column: u8, invert: bool| {
            let mut d = disp_static.borrow_mut();

            let y_px = (line as u16).saturating_mul(FONT_HEIGHT_PX);
            let x_px = (column as u16).saturating_mul(FONT_WIDTH_PX);
            if y_px < PANEL_HEIGHT {
                let h = FONT_HEIGHT_PX.min(PANEL_HEIGHT - y_px);
                let w = PANEL_WIDTH.saturating_sub(x_px);
                let clear_rect = Rectangle::new(
                    Point::new(x_px as i32, y_px as i32),
                    Size::new(w as u32, h as u32),
                );
                let clear_style = PrimitiveStyleBuilder::new()
                    .fill_color(BinaryColor::Off)
                    .build();
                let _ = clear_rect.into_styled(clear_style).draw(&mut *d);
            }

            let (fg, bg) = if invert {
                (BinaryColor::Off, BinaryColor::On)
            } else {
                (BinaryColor::On, BinaryColor::Off)
            };
            let text_style = MonoTextStyleBuilder::new()
                .font(&FONT_6X10)
                .text_color(fg)
                .background_color(bg)
                .build();

            Text::with_baseline(
                text,
                Point::new(x_px as i32, y_px as i32),
                text_style,
                Baseline::Top,
            )
            .draw(&mut *d)
            .map_err(|e| alloc::format!("display draw_text: {:?}", e))?;

            d.flush()
                .map_err(|e| alloc::format!("display draw_text flush: {:?}", e))
        });

    let draw_text_xy_fn: soma_esp32_port_display::DrawTextXyFn =
        Box::new(move |text: &str, x: u16, y: u16, invert: bool| {
            let mut d = disp_static.borrow_mut();
            let (fg, bg) = if invert {
                (BinaryColor::Off, BinaryColor::On)
            } else {
                (BinaryColor::On, BinaryColor::Off)
            };
            let text_style = MonoTextStyleBuilder::new()
                .font(&FONT_6X10)
                .text_color(fg)
                .background_color(bg)
                .build();
            Text::with_baseline(
                text,
                Point::new(x as i32, y as i32),
                text_style,
                Baseline::Top,
            )
            .draw(&mut *d)
            .map_err(|e| alloc::format!("display draw_text_xy: {:?}", e))?;
            d.flush()
                .map_err(|e| alloc::format!("display draw_text_xy flush: {:?}", e))
        });

    let fill_rect_fn: soma_esp32_port_display::FillRectFn =
        Box::new(move |x: u16, y: u16, width: u16, height: u16, on: bool| {
            let mut d = disp_static.borrow_mut();
            let color = if on { BinaryColor::On } else { BinaryColor::Off };
            let rect = Rectangle::new(
                Point::new(x as i32, y as i32),
                Size::new(width as u32, height as u32),
            );
            let style = PrimitiveStyleBuilder::new().fill_color(color).build();
            rect.into_styled(style)
                .draw(&mut *d)
                .map_err(|e| alloc::format!("display fill_rect: {:?}", e))?;
            d.flush()
                .map_err(|e| alloc::format!("display fill_rect flush: {:?}", e))
        });

    let set_contrast_fn: soma_esp32_port_display::SetContrastFn = Box::new(move |value: u8| {
        use ssd1306::prelude::Brightness;
        let mut d = disp_static.borrow_mut();
        let brightness = match value {
            0..=31 => Brightness::DIMMEST,
            32..=95 => Brightness::DIM,
            96..=159 => Brightness::NORMAL,
            160..=223 => Brightness::BRIGHT,
            _ => Brightness::BRIGHTEST,
        };
        d.set_brightness(brightness)
            .map_err(|e| alloc::format!("display set_contrast: {:?}", e))
    });

    let flush_fn: soma_esp32_port_display::FlushFn = Box::new(move || {
        let mut d = disp_static.borrow_mut();
        d.flush()
            .map_err(|e| alloc::format!("display flush: {:?}", e))
    });

    let display_port = soma_esp32_port_display::DisplayPort::new(
        info_fn,
        clear_fn,
        draw_text_line_fn,
        draw_text_xy_fn,
        fill_rect_fn,
        set_contrast_fn,
        flush_fn,
    );
    composite.register(Box::new(display_port));
    println!(
        "[port] registered: display (ssd1306 128x64 @ 0x{:02x} on GPIO{}/GPIO{})",
        PANEL_I2C_ADDR, sda_gpio, scl_gpio
    );
}
