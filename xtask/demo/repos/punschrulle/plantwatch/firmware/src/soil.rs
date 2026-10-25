//! The analog soil probe: the ADC reads its voltage, higher when drier.

use embassy_time::{Duration, Timer};
use esp_hal::Blocking;
use esp_hal::analog::adc::{Adc, AdcChannel, AdcPin};
use esp_hal::peripherals::ADC1;

/// The mean of 16 readings a millisecond apart: one alone jumps around.
pub async fn read<P: AdcChannel>(
    adc: &mut Adc<'_, ADC1<'_>, Blocking>,
    pin: &mut AdcPin<P, ADC1<'_>>,
) -> u16 {
    let mut sum = 0u32;
    for _ in 0..16 {
        sum += u32::from(nb::block!(adc.read_oneshot(pin)).unwrap_or(0));
        Timer::after(Duration::from_millis(1)).await;
    }
    (sum / 16) as u16
}
