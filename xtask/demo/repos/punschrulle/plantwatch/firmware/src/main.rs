#![no_std]
#![no_main]

mod board;
mod publish;
mod soil;

use embassy_executor::Spawner;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::rtc_cntl::{Rtc, sleep::TimerWakeupSource};
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;

/// How long a board sleeps between readings.
const SLEEP: core::time::Duration = core::time::Duration::from_secs(15 * 60);

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_hal_embassy::init(timg0.timer0);
    let board = board::this();

    let mut adc_config = AdcConfig::new();
    let mut pin = adc_config.enable_pin(p.GPIO2, Attenuation::_11dB);
    let mut adc = Adc::new(p.ADC1, adc_config);
    let raw = soil::read(&mut adc, &mut pin).await;
    println!("{}: raw {raw}", board.id);

    let stack = publish::connect(spawner, p.WIFI, timg0.timer1).await;
    if let Err(e) = publish::reading(stack, board, raw).await {
        println!("publishing failed: {e:?}");
    }

    let mut rtc = Rtc::new(p.LPWR);
    rtc.sleep_deep(&[&TimerWakeupSource::new(SLEEP)]);
}
