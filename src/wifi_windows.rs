//! Read-only WLAN discovery, nearby-network scans and narrowly scoped, hidden
//! connection requests. No profile XML or credentials are read by this module.

use crate::wifi_auto_connect::{AutoConnectRule, DiscoveredAdapter, WifiBackend, WifiObservation};
use serde_json::{json, Value};
use std::{
    ffi::c_void,
    mem::size_of,
    process::Stdio,
    ptr, thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use windows::{
    core::HSTRING,
    Security::Authorization::AppCapabilityAccess::{AppCapability, AppCapabilityAccessStatus},
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::HANDLE,
        NetworkManagement::{
            IpHelper::{ConvertInterfaceGuidToLuid, ConvertInterfaceLuidToAlias},
            Ndis::NET_LUID_LH,
            WiFi::{
                dot11_radio_state_on, wlan_interface_state_associating,
                wlan_interface_state_authenticating, wlan_interface_state_connected,
                wlan_interface_state_disconnecting, wlan_interface_state_discovering,
                wlan_interface_state_not_ready, wlan_intf_opcode_current_connection,
                dot11_BSS_type_any, wlan_intf_opcode_radio_state, WlanCloseHandle,
                WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
                WlanGetNetworkBssList, WlanGetProfileList, WlanOpenHandle, WlanQueryInterface,
                WlanScan, DOT11_SSID, WLAN_AVAILABLE_NETWORK_CONNECTED,
                WLAN_AVAILABLE_NETWORK_HAS_PROFILE, WLAN_AVAILABLE_NETWORK_LIST, WLAN_BSS_LIST,
                WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST,
                WLAN_PROFILE_INFO_LIST, WLAN_RADIO_STATE,
            },
        },
    },
};

pub struct WindowsWifi;

// On recent Windows releases querying the current WLAN connection can open a
// one-time location-consent dialog. CheckAccess is read-only and explicitly
// does not request access, so background workers never trigger that dialog.
fn ensure_wifi_access() -> Result<(), String> {
    let capability = AppCapability::Create(&HSTRING::from("wiFiControl"))
        .map_err(|_| "Cannot check Windows Wi-Fi access without prompting.".to_string())?;
    let access = capability
        .CheckAccess()
        .map_err(|_| "Cannot check Windows Wi-Fi access without prompting.".to_string())?;
    if access == AppCapabilityAccessStatus::Allowed {
        Ok(())
    } else if access == AppCapabilityAccessStatus::NotDeclaredByApp {
        Err("Windows cannot grant this app access to Wi-Fi details. Auto-connect is paused without opening a permission prompt.".into())
    } else {
        Err("Windows Wi-Fi access is not allowed. Check Location access in Windows Settings; no permission prompt was opened.".into())
    }
}

struct WlanClient(HANDLE);
impl WlanClient {
    fn open() -> Result<Self, String> {
        let mut version = 0;
        let mut handle = ptr::null_mut();
        let code = unsafe { WlanOpenHandle(2, ptr::null(), &mut version, &mut handle) };
        if code != 0 || handle.is_null() {
            return Err(format!(
                "Windows WLAN service is unavailable (error {code})."
            ));
        }
        Ok(Self(handle))
    }
}
impl Drop for WlanClient {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.0, ptr::null());
        }
    }
}

struct WlanBuffer(*mut c_void);
impl Drop for WlanBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WlanFreeMemory(self.0);
            }
        }
    }
}

fn utf16_string(value: &[u16]) -> String {
    let end = value.iter().position(|c| *c == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

fn guid_string(guid: &GUID) -> String {
    Uuid::from_fields(guid.data1, guid.data2, guid.data3, &guid.data4).to_string()
}

fn same_guid(left: &str, right: &str) -> bool {
    match (
        Uuid::parse_str(left.trim_matches(|c| c == '{' || c == '}')),
        Uuid::parse_str(right.trim_matches(|c| c == '{' || c == '}')),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn alias_for_guid(guid: &GUID) -> Result<String, String> {
    let mut luid = NET_LUID_LH { Value: 0 };
    let code = unsafe { ConvertInterfaceGuidToLuid(guid, &mut luid) };
    if code != 0 {
        return Err(format!("Cannot identify the Wi-Fi adapter (error {code})."));
    }
    let mut buffer = [0u16; 257];
    let code = unsafe { ConvertInterfaceLuidToAlias(&luid, buffer.as_mut_ptr(), buffer.len()) };
    if code != 0 {
        return Err(format!(
            "Cannot resolve the Wi-Fi adapter name (error {code})."
        ));
    }
    let alias = utf16_string(&buffer);
    crate::valid_interface_name(&alias).map_err(|e| e.message)
}

fn profiles(client: &WlanClient, guid: &GUID) -> Result<Vec<String>, String> {
    let mut list: *mut WLAN_PROFILE_INFO_LIST = ptr::null_mut();
    let code = unsafe { WlanGetProfileList(client.0, guid, ptr::null(), &mut list) };
    let _owned = WlanBuffer(list.cast());
    if code != 0 || list.is_null() {
        return Err(format!("Cannot read saved Wi-Fi profiles (error {code})."));
    }
    let count = unsafe { (*list).dwNumberOfItems as usize };
    if count > 4096 {
        return Err("Windows returned too many Wi-Fi profiles.".into());
    }
    let entries = unsafe { std::slice::from_raw_parts((*list).ProfileInfo.as_ptr(), count) };
    Ok(entries
        .iter()
        .map(|p| utf16_string(&p.strProfileName))
        .filter(|s| !s.is_empty())
        .collect())
}

fn query_copy<T: Copy>(client: &WlanClient, guid: &GUID, opcode: i32) -> Result<T, String> {
    let mut size = 0u32;
    let mut data: *mut c_void = ptr::null_mut();
    let code = unsafe {
        WlanQueryInterface(
            client.0,
            guid,
            opcode,
            ptr::null(),
            &mut size,
            &mut data,
            ptr::null_mut(),
        )
    };
    let _owned = WlanBuffer(data);
    if code != 0 || data.is_null() || (size as usize) < size_of::<T>() {
        return Err(format!("Cannot read Wi-Fi adapter state (error {code})."));
    }
    Ok(unsafe { ptr::read_unaligned(data.cast::<T>()) })
}

struct InspectedAdapter {
    view: DiscoveredAdapter,
    state: i32,
    radio_on: Result<bool, String>,
}

fn inspect_info(
    client: &WlanClient,
    info: &WLAN_INTERFACE_INFO,
) -> Result<InspectedAdapter, String> {
    let alias = alias_for_guid(&info.InterfaceGuid)?;
    let connected = info.isState == wlan_interface_state_connected;
    let current_profile = if connected {
        let connection: WLAN_CONNECTION_ATTRIBUTES = query_copy(
            client,
            &info.InterfaceGuid,
            wlan_intf_opcode_current_connection,
        )?;
        let name = utf16_string(&connection.strProfileName);
        (!name.is_empty()).then_some(name)
    } else {
        None
    };
    let radio_on =
        query_copy::<WLAN_RADIO_STATE>(client, &info.InterfaceGuid, wlan_intf_opcode_radio_state)
            .map(|radio| {
                let count = (radio.dwNumberOfPhys as usize).min(radio.PhyRadioState.len());
                count > 0
                    && radio.PhyRadioState[..count].iter().any(|phy| {
                        phy.dot11SoftwareRadioState == dot11_radio_state_on
                            && phy.dot11HardwareRadioState == dot11_radio_state_on
                    })
            });
    Ok(InspectedAdapter {
        view: DiscoveredAdapter {
            adapter_guid: guid_string(&info.InterfaceGuid),
            interface_name: alias,
            connected,
            current_profile,
            profiles: profiles(client, &info.InterfaceGuid)?,
        },
        state: info.isState,
        radio_on,
    })
}

fn interface_infos(client: &WlanClient) -> Result<Vec<WLAN_INTERFACE_INFO>, String> {
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = ptr::null_mut();
    let code = unsafe { WlanEnumInterfaces(client.0, ptr::null(), &mut list) };
    let _owned = WlanBuffer(list.cast());
    if code != 0 || list.is_null() {
        return Err(format!("Cannot enumerate Wi-Fi adapters (error {code})."));
    }
    let count = unsafe { (*list).dwNumberOfItems as usize };
    if count > 1024 {
        return Err("Windows returned too many Wi-Fi adapters.".into());
    }
    let entries = unsafe { std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count) };
    Ok(entries.to_vec())
}

pub fn discover_adapters() -> Result<Vec<DiscoveredAdapter>, String> {
    ensure_wifi_access()?;
    let client = WlanClient::open()?;
    let mut adapters = Vec::new();
    let mut first_error = None;
    for info in interface_infos(&client)? {
        match inspect_info(&client, &info) {
            Ok(adapter) => adapters.push(adapter.view),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if adapters.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    Ok(adapters)
}

fn selected_adapter(rule: &AutoConnectRule) -> Result<InspectedAdapter, String> {
    ensure_wifi_access()?;
    let client = WlanClient::open()?;
    let info = interface_infos(&client)?
        .into_iter()
        .find(|info| same_guid(&guid_string(&info.InterfaceGuid), &rule.adapter_guid))
        .ok_or_else(|| "The configured Wi-Fi adapter is unavailable.".to_string())?;
    let adapter = inspect_info(&client, &info)?;
    if !adapter
        .view
        .profiles
        .iter()
        .any(|name| name == &rule.profile_name)
    {
        return Err("The selected saved Wi-Fi profile is unavailable on this adapter.".into());
    }
    if adapter.state == wlan_interface_state_not_ready
        || !*adapter.radio_on.as_ref().map_err(Clone::clone)?
    {
        return Err("The Wi-Fi adapter or radio is not ready.".into());
    }
    if [
        wlan_interface_state_associating,
        wlan_interface_state_authenticating,
        wlan_interface_state_disconnecting,
        wlan_interface_state_discovering,
    ]
    .contains(&adapter.state)
    {
        return Err("The Wi-Fi adapter is busy changing connections.".into());
    }
    Ok(adapter)
}

/// Band label and channel number for a centre frequency in kHz.
pub fn band_channel(frequency_khz: u32) -> (Option<&'static str>, Option<u32>) {
    let mhz = frequency_khz / 1000;
    match mhz {
        2484 => (Some("2.4 GHz"), Some(14)),
        2412..=2472 => (Some("2.4 GHz"), Some((mhz - 2407) / 5)),
        5955..=7115 => (Some("6 GHz"), Some((mhz - 5950) / 5)),
        5000..=5925 => (Some("5 GHz"), Some((mhz - 5000) / 5)),
        _ => (None, None),
    }
}

/// DOT11_AUTH_ALGORITHM as a short security label.
pub fn security_text(algorithm: i32, secured: bool) -> &'static str {
    match algorithm {
        1 if !secured => "Open",
        1 | 2 => "WEP",
        3 => "WPA-Enterprise",
        4 => "WPA-Personal",
        6 => "WPA2-Enterprise",
        7 => "WPA2-Personal",
        8 | 11 => "WPA3-Enterprise",
        9 => "WPA3-Personal",
        10 => "OWE",
        _ => "Other",
    }
}

fn ssid_text(ssid: &DOT11_SSID) -> String {
    let length = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
    String::from_utf8_lossy(&ssid.ucSSID[..length]).into_owned()
}

/// Scan for nearby networks with one adapter and list every visible access
/// point. This only reads what Windows sees; it never connects.
pub fn scan_networks(adapter_guid: &str) -> Result<Value, String> {
    ensure_wifi_access()?;
    let client = WlanClient::open()?;
    let info = interface_infos(&client)?
        .into_iter()
        .find(|info| same_guid(&guid_string(&info.InterfaceGuid), adapter_guid))
        .ok_or_else(|| "The selected Wi-Fi adapter is unavailable.".to_string())?;
    let guid = info.InterfaceGuid;
    // A fresh scan completes within a few seconds; when Windows refuses one
    // (e.g. while connecting) the recent cached results are used instead.
    let scanned = unsafe { WlanScan(client.0, &guid, ptr::null(), ptr::null(), ptr::null()) } == 0;
    if scanned {
        thread::sleep(Duration::from_millis(4000));
    }
    let mut networks: Vec<(String, bool, i32, bool, bool)> = vec![];
    let mut available: *mut WLAN_AVAILABLE_NETWORK_LIST = ptr::null_mut();
    let code = unsafe { WlanGetAvailableNetworkList(client.0, &guid, 0, ptr::null(), &mut available) };
    let _available = WlanBuffer(available.cast());
    if code == 0 && !available.is_null() {
        let count = unsafe { (*available).dwNumberOfItems as usize }.min(4096);
        let items = unsafe { std::slice::from_raw_parts((*available).Network.as_ptr(), count) };
        for item in items {
            networks.push((
                ssid_text(&item.dot11Ssid),
                item.bSecurityEnabled != 0,
                item.dot11DefaultAuthAlgorithm,
                item.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0,
                item.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
            ));
        }
    }
    let mut list: *mut WLAN_BSS_LIST = ptr::null_mut();
    let code = unsafe {
        WlanGetNetworkBssList(client.0, &guid, ptr::null(), dot11_BSS_type_any, 0, ptr::null(), &mut list)
    };
    let _list = WlanBuffer(list.cast());
    if code != 0 || list.is_null() {
        return Err(format!("Cannot read nearby Wi-Fi networks (error {code})."));
    }
    let count = unsafe { (*list).dwNumberOfItems as usize }.min(4096);
    let entries = unsafe { std::slice::from_raw_parts((*list).wlanBssEntries.as_ptr(), count) };
    let access_points = entries
        .iter()
        .map(|entry| {
            let ssid = ssid_text(&entry.dot11Ssid);
            let known = networks.iter().find(|(name, ..)| name == &ssid);
            let (band, channel) = band_channel(entry.ulChCenterFrequency);
            let bssid = entry.dot11Bssid.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
            json!({
                "ssid":ssid,
                "bssid":bssid,
                "rssi":entry.lRssi,
                "signalPercent":entry.uLinkQuality.min(100),
                "frequencyMhz":entry.ulChCenterFrequency / 1000,
                "band":band,
                "channel":channel,
                "security":known.map(|(_, secured, algorithm, ..)| security_text(*algorithm, *secured)),
                "savedProfile":known.map(|(.., saved, _)| *saved).unwrap_or(false),
                "connected":known.map(|(.., connected)| *connected).unwrap_or(false)
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "adapterGuid":guid_string(&guid),
        "interfaceName":alias_for_guid(&guid).unwrap_or_default(),
        "fresh":scanned,
        "scannedAt":chrono::Utc::now().to_rfc3339(),
        "accessPoints":access_points
    }))
}

/// The current connection of one Wi-Fi adapter, for background monitoring.
pub struct WifiConnection {
    pub adapter_guid: String,
    pub alias: String,
    pub ssid: Option<String>,
    pub profile: Option<String>,
    pub signal: Option<u32>,
}

/// Current connection and signal of every Wi-Fi adapter. Uses the same
/// no-prompt access check as auto-connect and never scans or connects.
pub fn connection_samples() -> Result<Vec<WifiConnection>, String> {
    ensure_wifi_access()?;
    let client = WlanClient::open()?;
    Ok(interface_infos(&client)?
        .into_iter()
        .map(|info| {
            let connection = (info.isState == wlan_interface_state_connected)
                .then(|| {
                    query_copy::<WLAN_CONNECTION_ATTRIBUTES>(
                        &client,
                        &info.InterfaceGuid,
                        wlan_intf_opcode_current_connection,
                    )
                    .ok()
                })
                .flatten();
            WifiConnection {
                adapter_guid: guid_string(&info.InterfaceGuid),
                alias: alias_for_guid(&info.InterfaceGuid).unwrap_or_default(),
                ssid: connection
                    .as_ref()
                    .map(|item| ssid_text(&item.wlanAssociationAttributes.dot11Ssid))
                    .filter(|ssid| !ssid.is_empty()),
                profile: connection
                    .as_ref()
                    .map(|item| utf16_string(&item.strProfileName))
                    .filter(|name| !name.is_empty()),
                signal: connection.as_ref().map(|item| item.wlanAssociationAttributes.wlanSignalQuality.min(100)),
            }
        })
        .collect())
}

impl WifiBackend for WindowsWifi {
    fn observe(&self, rule: &AutoConnectRule) -> Result<WifiObservation, String> {
        let adapter = selected_adapter(rule)?;
        Ok(WifiObservation {
            adapter_guid: adapter.view.adapter_guid,
            connected: adapter.view.connected,
            current_profile: adapter.view.current_profile,
        })
    }

    fn connect(&self, rule: &AutoConnectRule) -> Result<(), String> {
        // Resolve the adapter again immediately before launching the fixed command.
        let adapter = selected_adapter(rule)?;
        if adapter.view.connected
            && adapter.view.current_profile.as_deref() == Some(rule.profile_name.as_str())
        {
            return Ok(());
        }
        let mut child = crate::hidden_command("netsh.exe")
            .args([
                "wlan",
                "connect",
                &format!("name={}", rule.profile_name),
                &format!("interface={}", adapter.view.interface_name),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Cannot start the Wi-Fi connection request: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    return Err(format!("The Wi-Fi connection request failed ({status})."))
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("The Wi-Fi connection request timed out.".into());
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "Cannot wait for the Wi-Fi connection request: {error}"
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guid_and_unicode_names_are_preserved() {
        let guid = GUID::from_u128(0x255bc7a7_a80c_47e6_902b_386aa6ee90db);
        assert!(same_guid(
            &guid_string(&guid),
            "{255BC7A7-A80C-47E6-902B-386AA6EE90DB}"
        ));
        assert!(!same_guid(&guid_string(&guid), "invalid"));
        let name: Vec<_> = "转化医学813\0ignored".encode_utf16().collect();
        assert_eq!(utf16_string(&name), "转化医学813");
    }

    #[test]
    fn frequencies_map_to_bands_and_channels() {
        assert_eq!(band_channel(2_412_000), (Some("2.4 GHz"), Some(1)));
        assert_eq!(band_channel(2_437_000), (Some("2.4 GHz"), Some(6)));
        assert_eq!(band_channel(2_484_000), (Some("2.4 GHz"), Some(14)));
        assert_eq!(band_channel(5_180_000), (Some("5 GHz"), Some(36)));
        assert_eq!(band_channel(5_825_000), (Some("5 GHz"), Some(165)));
        assert_eq!(band_channel(5_955_000), (Some("6 GHz"), Some(1)));
        assert_eq!(band_channel(60_480_000), (None, None));
        assert_eq!(security_text(7, true), "WPA2-Personal");
        assert_eq!(security_text(9, true), "WPA3-Personal");
        assert_eq!(security_text(1, false), "Open");
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(b"Home");
        assert_eq!(ssid_text(&DOT11_SSID { uSSIDLength: 4, ucSSID: bytes }), "Home");
        assert_eq!(ssid_text(&DOT11_SSID { uSSIDLength: 99, ucSSID: bytes }).len(), 32);
    }

    #[test]
    #[ignore = "manual, read-only check of local Windows WLAN discovery"]
    fn read_only_windows_wlan_discovery() {
        let adapters = discover_adapters().expect("read-only WLAN discovery should succeed");
        assert!(!adapters.is_empty(), "no WLAN interfaces were discovered");
        assert!(adapters.iter().all(|adapter| {
            Uuid::parse_str(&adapter.adapter_guid).is_ok() && !adapter.interface_name.is_empty()
        }));
    }
}
