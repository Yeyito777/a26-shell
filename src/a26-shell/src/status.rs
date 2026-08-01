use std::fs;

const BATTERY_CAPACITY: &str = "/sys/class/power_supply/battery/capacity";
const BATTERY_STATUS: &str = "/sys/class/power_supply/battery/status";
const WIFI_OPERSTATE: &str = "/sys/class/net/wlan0/operstate";
const WIFI_CARRIER: &str = "/sys/class/net/wlan0/carrier";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceStatus {
    pub battery_percent: Option<u8>,
    pub battery_charging: bool,
    pub wifi_connected: bool,
}

impl DeviceStatus {
    pub fn read() -> Self {
        let battery_percent = fs::read_to_string(BATTERY_CAPACITY)
            .ok()
            .and_then(|value| value.trim().parse::<u8>().ok())
            .map(|value| value.min(100));
        let battery_charging =
            fs::read_to_string(BATTERY_STATUS).is_ok_and(|value| value.trim() == "Charging");
        let wifi_connected = fs::read_to_string(WIFI_OPERSTATE)
            .is_ok_and(|value| value.trim() == "up")
            && fs::read_to_string(WIFI_CARRIER).is_ok_and(|value| value.trim() == "1");
        Self {
            battery_percent,
            battery_charging,
            wifi_connected,
        }
    }
}
