//! Which plant is thirsty.

use crate::db::Calibration;

/// How wet a pot's soil is, from 0 (as dry as when calibrated dry) to 100
/// (as wet as just after watering); `None` until both are calibrated.
pub fn percent(raw: u16, cal: Calibration) -> Option<u8> {
    let (dry, wet) = (f32::from(cal.dry?), f32::from(cal.wet?));
    if (dry - wet).abs() < 1.0 {
        return None;
    }
    let p = (f32::from(raw) - dry) / (wet - dry) * 100.0;
    Some(p.clamp(0.0, 100.0).round() as u8)
}

/// Thirsty below a third.
pub fn thirsty(percent: u8) -> bool {
    percent < 33
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONSTERA: Calibration = Calibration { dry: Some(3050), wet: Some(1420) };

    #[test]
    fn readings_turn_into_percent() {
        assert_eq!(percent(3050, MONSTERA), Some(0));
        assert_eq!(percent(1420, MONSTERA), Some(100));
        assert_eq!(percent(2235, MONSTERA), Some(50));
    }

    #[test]
    fn beyond_the_calibration_is_clamped() {
        assert_eq!(percent(3300, MONSTERA), Some(0));
        assert_eq!(percent(900, MONSTERA), Some(100));
    }

    #[test]
    fn nothing_without_both() {
        assert_eq!(percent(2000, Calibration { dry: Some(3050), wet: None }), None);
    }

    #[test]
    fn a_third_is_thirsty() {
        assert!(thirsty(32));
        assert!(!thirsty(33));
    }
}
