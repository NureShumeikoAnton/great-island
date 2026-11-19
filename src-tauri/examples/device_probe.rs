//! Probe for devices (TASK_06), without the island. Read-only: it listens and writes down.
//!
//! While it runs it records what Windows tells about devices: paired Bluetooth devices
//! (connected, disconnected, every property), their battery, the device containers, the sound
//! endpoints and the default output / input, the BLE advertisements of headphone makers and the
//! gamepads. Each line goes to the console and to `device_probe.log` next to the exe (or in the
//! folder given as the first argument).
//!
//! Type a note and Enter to put a marker into the log ("connecting the buds"); `q` ends.

use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{BufRead, Write},
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        mpsc::{Sender, channel},
    },
    time::{Duration, Instant},
};
use windows::{
    Devices::{
        Bluetooth::{
            Advertisement::{
                BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
                BluetoothLEScanningMode,
            },
            BluetoothCacheMode, BluetoothDevice, BluetoothLEDevice,
            GenericAttributeProfile::{GattCommunicationStatus, GattDeviceService},
        },
        Enumeration::{
            DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher,
        },
    },
    Foundation::{EventHandler, IPropertyValue, PropertyType, TypedEventHandler},
    Gaming::Input::RawGameController,
    Storage::Streams::{DataReader, IBuffer},
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            DEVICE_STATE, DEVICE_STATEMASK_ALL, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator,
            IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eAll, eCapture,
            eCommunications, eConsole, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            STGM_READ,
            StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
        },
    },
    core::{GUID, HSTRING, IInspectable, Interface, PCWSTR, Result},
};
use windows_collections::IIterable;
use windows_core::implement;

struct Log {
    file: Option<File>,
    started: Instant,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

fn log(source: &str, text: impl AsRef<str>) {
    let Some(lock) = LOG.get() else { return };
    let mut log = lock.lock().unwrap();
    let ms = log.started.elapsed().as_millis();
    let line = format!(
        "{:>4}.{:03} [{source}] {}",
        ms / 1000,
        ms % 1000,
        text.as_ref()
    );
    println!("{line}");
    if let Some(file) = log.file.as_mut() {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

fn log_err(source: &str, what: &str, e: windows::core::Error) {
    log(
        source,
        format!("{what}: error {:#010x} {}", e.code().0, e.message()),
    );
}

fn clock() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn buffer_bytes(buffer: &IBuffer) -> Result<Vec<u8>> {
    let len = buffer.Length()? as usize;
    let reader = DataReader::FromBuffer(buffer)?;
    let mut bytes = vec![0u8; len];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

fn value_text(value: &IInspectable) -> String {
    let Ok(v) = value.cast::<IPropertyValue>() else {
        return "<object>".into();
    };
    let Ok(kind) = v.Type() else {
        return "<?>".into();
    };
    let text = match kind {
        PropertyType::String => v.GetString().map(|s| format!("\"{s}\"")),
        PropertyType::Boolean => v.GetBoolean().map(|b| b.to_string()),
        PropertyType::UInt8 => v.GetUInt8().map(|n| n.to_string()),
        PropertyType::Int16 => v.GetInt16().map(|n| n.to_string()),
        PropertyType::UInt16 => v.GetUInt16().map(|n| format!("{n} (0x{n:04x})")),
        PropertyType::Int32 => v.GetInt32().map(|n| n.to_string()),
        PropertyType::UInt32 => v.GetUInt32().map(|n| format!("{n} (0x{n:08x})")),
        PropertyType::Int64 => v.GetInt64().map(|n| n.to_string()),
        PropertyType::UInt64 => v.GetUInt64().map(|n| format!("{n} (0x{n:x})")),
        PropertyType::Double => v.GetDouble().map(|n| n.to_string()),
        PropertyType::Guid => v.GetGuid().map(|g| format!("{g:?}")),
        PropertyType::DateTime => v
            .GetDateTime()
            .map(|d| format!("datetime {}", d.UniversalTime)),
        PropertyType::StringArray => {
            let mut a = windows_core::Array::<HSTRING>::new();
            v.GetStringArray(&mut a).map(|_| {
                let items: Vec<String> = a.iter().map(|s| format!("\"{s}\"")).collect();
                format!("[{}]", items.join(", "))
            })
        }
        PropertyType::UInt8Array => {
            let mut a = windows_core::Array::<u8>::new();
            v.GetUInt8Array(&mut a)
                .map(|_| format!("bytes[{}] {}", a.len(), hex(&a)))
        }
        other => Ok(format!("<type {}>", other.0)),
    };
    text.unwrap_or_else(|e| format!("<unreadable {:#x}>", e.code().0))
}

fn props_text(props: &windows_collections::IMapView<HSTRING, IInspectable>) -> Vec<String> {
    let mut lines = Vec::new();
    let Ok(it) = props.First() else { return lines };
    while it.HasCurrent().unwrap_or(false) {
        if let Ok(pair) = it.Current() {
            let key = pair.Key().map(|k| k.to_string()).unwrap_or_default();
            let value = match pair.Value() {
                Ok(v) => value_text(&v),
                Err(_) => "null".into(),
            };
            if value != "null" {
                lines.push(format!("    {key} = {value}"));
            }
        }
        if !it.MoveNext().unwrap_or(false) {
            break;
        }
    }
    lines.sort();
    lines
}

/// Battery Windows reads from hands-free headsets (`DEVPKEY_Bluetooth_Battery`), the same
/// value its Bluetooth settings page shows.
const BT_BATTERY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";

const AEP_PROPS: &[&str] = &[
    "System.ItemNameDisplay",
    "System.Devices.Aep.IsConnected",
    "System.Devices.Aep.IsPaired",
    "System.Devices.Aep.IsPresent",
    "System.Devices.Aep.DeviceAddress",
    "System.Devices.Aep.ProtocolId",
    "System.Devices.Aep.ContainerId",
    "System.Devices.Aep.Category",
    "System.Devices.Aep.Manufacturer",
    "System.Devices.Aep.ModelName",
    "System.Devices.Aep.ModelId",
    "System.Devices.Aep.SignalStrength",
    "System.Devices.Aep.Bluetooth.Cod.Major",
    "System.Devices.Aep.Bluetooth.Cod.Minor",
    "System.Devices.Aep.Bluetooth.Cod.Services.Audio",
    "System.Devices.Aep.Bluetooth.Cod.Services.Rendering",
    "System.Devices.Aep.Bluetooth.Le.Appearance",
    "System.Devices.Aep.Bluetooth.Le.Appearance.Category",
    "System.Devices.Aep.Bluetooth.Le.Appearance.Subcategory",
    "System.Devices.Aep.Bluetooth.Le.IsConnectable",
    "System.Devices.Aep.Bluetooth.LastSeenTime",
];

const DEVICE_PROPS: &[&str] = &[
    "System.ItemNameDisplay",
    "System.Devices.DeviceInstanceId",
    "System.Devices.HardwareIds",
    "System.Devices.CompatibleIds",
    "System.Devices.ClassGuid",
    "System.Devices.ContainerId",
    "System.Devices.Parent",
    "System.Devices.Present",
    "System.Devices.Manufacturer",
    "System.Devices.DeviceManufacturer",
    "System.Devices.ModelName",
    "System.Devices.ModelNumber",
    "System.Devices.BusReportedDeviceDesc",
    BT_BATTERY,
];

const CONTAINER_PROPS: &[&str] = &[
    "System.ItemNameDisplay",
    "System.Devices.Category",
    "System.Devices.CategoryIds",
    "System.Devices.Connected",
    "System.Devices.Paired",
    "System.Devices.Present",
    "System.Devices.IsDefault",
    "System.Devices.Manufacturer",
    "System.Devices.ModelName",
    "System.Devices.ModelNumber",
    "System.Devices.FriendlyName",
    "System.Devices.Icon",
    "System.Devices.Icons",
    "System.Devices.LaunchDeviceStageFromExplorer",
    BT_BATTERY,
];

/// Keeps only the property names Windows accepts for that kind: one unknown name makes the
/// whole watcher fail. Creating a watcher checks the names without searching (a query over
/// Bluetooth endpoints would scan the air for tens of seconds).
fn accepted(source: &str, kind: DeviceInformationKind, aqs: &str, names: &[&str]) -> Vec<HSTRING> {
    let mut ok = Vec::new();
    for name in names {
        let list: IIterable<HSTRING> = vec![HSTRING::from(*name)].into();
        match DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
            &HSTRING::from(aqs),
            &list,
            kind,
        ) {
            Ok(_) => ok.push(HSTRING::from(*name)),
            Err(e) => log(
                source,
                format!("property {name} not accepted ({:#x})", e.code().0),
            ),
        }
    }
    ok
}

struct Watch {
    source: &'static str,
    names: Mutex<HashMap<String, String>>,
    enumerated: Mutex<bool>,
    /// Logs an added device during the first enumeration only when this returns true.
    snapshot_filter: fn(&str, &[String]) -> bool,
    on_connected: Option<Sender<String>>,
}

static WATCHES: OnceLock<Mutex<Vec<DeviceWatcher>>> = OnceLock::new();

fn watch(
    source: &'static str,
    kind: DeviceInformationKind,
    aqs: &str,
    props: &[&str],
    snapshot_filter: fn(&str, &[String]) -> bool,
    on_connected: Option<Sender<String>>,
) {
    let names = accepted(source, kind, aqs, props);
    let list: IIterable<HSTRING> = names.into();
    let watcher = match DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(aqs),
        &list,
        kind,
    ) {
        Ok(w) => w,
        Err(e) => return log_err(source, "watcher", e),
    };
    let state: &'static Watch = Box::leak(Box::new(Watch {
        source,
        names: Mutex::new(HashMap::new()),
        enumerated: Mutex::new(false),
        snapshot_filter,
        on_connected,
    }));

    let added = watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
        move |_, info| {
            let Ok(info) = info.ok() else { return Ok(()) };
            let id = info.Id()?.to_string();
            let name = info.Name()?.to_string();
            state.names.lock().unwrap().insert(id.clone(), name.clone());
            let lines = info
                .Properties()
                .map(|p| props_text(&p))
                .unwrap_or_default();
            let live = *state.enumerated.lock().unwrap();
            if !live && !(state.snapshot_filter)(&id, &lines) {
                return Ok(());
            }
            let when = if live { "ADDED" } else { "present" };
            log(
                state.source,
                format!("{when} \"{name}\"\n    id = {id}\n{}", lines.join("\n")),
            );
            if let Some(tx) = &state.on_connected
                && lines.iter().any(|l| l.contains("IsConnected = true"))
            {
                let _ = tx.send(id);
            }
            Ok(())
        },
    ));
    let updated = watcher.Updated(
        &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, update| {
            let Ok(update) = update.ok() else {
                return Ok(());
            };
            let id = update.Id()?.to_string();
            let name = state
                .names
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or_default();
            let lines = update
                .Properties()
                .map(|p| props_text(&p))
                .unwrap_or_default();
            if lines.is_empty() {
                return Ok(());
            }
            log(
                state.source,
                format!("UPDATED \"{name}\" ({id})\n{}", lines.join("\n")),
            );
            if let Some(tx) = &state.on_connected
                && lines.iter().any(|l| l.contains("IsConnected = true"))
            {
                let _ = tx.send(id);
            }
            Ok(())
        }),
    );
    let removed = watcher.Removed(
        &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, update| {
            let Ok(update) = update.ok() else {
                return Ok(());
            };
            let id = update.Id()?.to_string();
            let name = state.names.lock().unwrap().remove(&id).unwrap_or_default();
            log(state.source, format!("REMOVED \"{name}\" ({id})"));
            Ok(())
        }),
    );
    let completed = watcher.EnumerationCompleted(
        &TypedEventHandler::<DeviceWatcher, IInspectable>::new(move |_, _| {
            *state.enumerated.lock().unwrap() = true;
            let count = state.names.lock().unwrap().len();
            log(
                state.source,
                format!("enumeration done, {count} known; live events from here"),
            );
            Ok(())
        }),
    );
    let stopped = watcher.Stopped(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(
        move |w, _| {
            let status = w.ok().and_then(|w| w.Status()).map(|s| s.0).unwrap_or(-1);
            log(
                state.source,
                format!("WATCHER STOPPED, status {status} (5 = aborted)"),
            );
            Ok(())
        },
    ));
    for (what, r) in [
        ("Added", added),
        ("Updated", updated),
        ("Removed", removed),
        ("Completed", completed),
        ("Stopped", stopped),
    ] {
        if let Err(e) = r {
            log_err(source, what, e);
        }
    }
    if let Err(e) = watcher.Start() {
        return log_err(source, "start", e);
    }
    WATCHES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(watcher);
}

fn every_device(_: &str, _: &[String]) -> bool {
    true
}

/// The first enumeration lists every device of the PC; only the ones a headphone, a mouse or a
/// gamepad could be are worth reading.
fn device_worth_logging(id: &str, lines: &[String]) -> bool {
    let id = id.to_ascii_uppercase();
    [
        "BTHENUM",
        "BTHLE",
        "BTHHFENUM",
        "BTHLEDEVICE",
        "USB\\VID",
        "HID\\",
        "SWD\\MMDEVAPI",
    ]
    .iter()
    .any(|p| id.starts_with(p))
        || lines.iter().any(|l| l.contains("104EA319"))
}

const BATTERY_SERVICE: GUID = GUID::from_u128(0x0000180f_0000_1000_8000_00805f9b34fb);
const BATTERY_LEVEL: GUID = GUID::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb);
const DEVICE_INFO_SERVICE: GUID = GUID::from_u128(0x0000180a_0000_1000_8000_00805f9b34fb);
const DEVICE_INFO: &[(u128, &str)] = &[
    (0x00002a29_0000_1000_8000_00805f9b34fb, "manufacturer"),
    (0x00002a24_0000_1000_8000_00805f9b34fb, "model number"),
    (0x00002a26_0000_1000_8000_00805f9b34fb, "firmware"),
    (0x00002a27_0000_1000_8000_00805f9b34fb, "hardware revision"),
];

fn bluetooth_details(id: String) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    if id.contains("BluetoothLE#") {
        if let Err(e) = le_details(&id) {
            log_err("bt-le", &format!("details of {id}"), e);
        }
    } else if let Err(e) = classic_details(&id) {
        log_err("bt", &format!("details of {id}"), e);
    }
}

fn classic_details(id: &str) -> Result<()> {
    let device = BluetoothDevice::FromIdAsync(&HSTRING::from(id))?.join()?;
    let cod = device.ClassOfDevice()?;
    let mut lines = vec![
        format!("    name = \"{}\"", device.Name()?),
        format!("    address = {:012x}", device.BluetoothAddress()?),
        format!("    connection = {:?}", device.ConnectionStatus()?.0),
        format!(
            "    class of device = 0x{:06x} major {} minor {} services 0x{:x}",
            cod.RawValue()?,
            cod.MajorClass()?.0,
            cod.MinorClass()?.0,
            cod.ServiceCapabilities()?.0
        ),
    ];
    for mode in [BluetoothCacheMode::Cached, BluetoothCacheMode::Uncached] {
        let result = device.GetRfcommServicesWithCacheModeAsync(mode)?.join()?;
        let services = result.Services()?;
        let mut found = Vec::new();
        for i in 0..services.Size()? {
            let s = services.GetAt(i)?;
            let uuid = s.ServiceId()?.Uuid()?;
            let name = s
                .ConnectionServiceName()
                .map(|n| n.to_string())
                .unwrap_or_default();
            found.push(format!("      {uuid:?} {name}"));
            let _ = s.Close();
        }
        lines.push(format!(
            "    rfcomm services ({}, {:?}): {}",
            if mode == BluetoothCacheMode::Cached {
                "cached"
            } else {
                "asked"
            },
            result.Error()?.0,
            found.len()
        ));
        lines.extend(found);
        if services.Size()? > 0 {
            break;
        }
    }
    log(
        "bt",
        format!("DETAILS (classic) {id}\n{}", lines.join("\n")),
    );
    let _ = device.Close();
    Ok(())
}

fn le_details(id: &str) -> Result<()> {
    let device = BluetoothLEDevice::FromIdAsync(&HSTRING::from(id))?.join()?;
    let look = device.Appearance()?;
    let mut lines = vec![
        format!("    name = \"{}\"", device.Name()?),
        format!("    address = {:012x}", device.BluetoothAddress()?),
        format!("    connection = {:?}", device.ConnectionStatus()?.0),
        format!(
            "    appearance = 0x{:04x} category {} subcategory {}",
            look.RawValue()?,
            look.Category()?,
            look.SubCategory()?
        ),
    ];
    let result = device
        .GetGattServicesWithCacheModeAsync(BluetoothCacheMode::Uncached)?
        .join()?;
    lines.push(format!("    gatt status = {:?}", result.Status()?.0));
    if result.Status()? == GattCommunicationStatus::Success {
        let services = result.Services()?;
        for i in 0..services.Size()? {
            let s = services.GetAt(i)?;
            let uuid = s.Uuid()?;
            lines.push(format!("      service {uuid:?}"));
            if uuid == BATTERY_SERVICE {
                lines.push(format!(
                    "        battery level = {}",
                    read_text(&s, BATTERY_LEVEL, true)
                ));
            }
            if uuid == DEVICE_INFO_SERVICE {
                for (char_id, label) in DEVICE_INFO {
                    lines.push(format!(
                        "        {label} = {}",
                        read_text(&s, GUID::from_u128(*char_id), false)
                    ));
                }
            }
            let _ = s.Close();
        }
    }
    log("bt-le", format!("DETAILS (LE) {id}\n{}", lines.join("\n")));
    let _ = device.Close();
    Ok(())
}

fn read_text(service: &GattDeviceService, char_id: GUID, number: bool) -> String {
    let read = || -> Result<String> {
        let chars = service
            .GetCharacteristicsForUuidWithCacheModeAsync(char_id, BluetoothCacheMode::Uncached)?
            .join()?
            .Characteristics()?;
        if chars.Size()? == 0 {
            return Ok("<absent>".into());
        }
        let value = chars
            .GetAt(0)?
            .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)?
            .join()?;
        if value.Status()? != GattCommunicationStatus::Success {
            return Ok(format!("<status {}>", value.Status()?.0));
        }
        let bytes = buffer_bytes(&value.Value()?)?;
        Ok(if number {
            format!("{:?}", bytes)
        } else {
            format!("\"{}\"", String::from_utf8_lossy(&bytes))
        })
    };
    read().unwrap_or_else(|e| format!("<error {:#x}>", e.code().0))
}

fn key(fmtid: u128, pid: u32) -> PROPERTYKEY {
    PROPERTYKEY {
        fmtid: GUID::from_u128(fmtid),
        pid,
    }
}

const ENDPOINT_PROPS: &[(&str, u128, u32)] = &[
    ("name", 0xa45c254e_df1c_4efd_8020_67d146a850e0, 14),
    ("description", 0xa45c254e_df1c_4efd_8020_67d146a850e0, 2),
    ("adapter", 0x026e516e_b814_414b_83cd_856d6fef4822, 2),
    ("form factor", 0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e, 0),
    ("jack subtype", 0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e, 8),
    ("container", 0x8c7ed206_3f8a_4827_b3ab_ae9e1faefc6c, 2),
    ("enumerator", 0xa45c254e_df1c_4efd_8020_67d146a850e0, 24),
    ("instance", 0x78c34fc8_104a_4aca_9ea4_524d52996e57, 256),
];

const FORM_FACTORS: &[&str] = &[
    "remote network device",
    "speakers",
    "line level",
    "headphones",
    "microphone",
    "headset",
    "handset",
    "digital passthrough",
    "spdif",
    "digital display (HDMI / DisplayPort)",
    "unknown",
];

fn flow_name(flow: EDataFlow) -> &'static str {
    match flow {
        f if f == eRender => "output",
        f if f == eCapture => "input",
        _ => "both",
    }
}

fn role_name(role: ERole) -> &'static str {
    match role {
        r if r == eConsole => "console",
        r if r == eCommunications => "communications",
        _ => "multimedia",
    }
}

fn state_name(state: DEVICE_STATE) -> String {
    match state.0 {
        1 => "active".into(),
        2 => "disabled".into(),
        4 => "not present".into(),
        8 => "unplugged".into(),
        n => format!("state {n}"),
    }
}

fn device_id(device: &IMMDevice) -> String {
    unsafe {
        let Ok(raw) = device.GetId() else {
            return String::new();
        };
        let id = raw.to_string().unwrap_or_default();
        CoTaskMemFree(Some(raw.0 as *const _));
        id
    }
}

fn describe_endpoint(devices: &IMMDeviceEnumerator, id: &str) -> String {
    let wide = HSTRING::from(id);
    let device = match unsafe { devices.GetDevice(PCWSTR(wide.as_ptr())) } {
        Ok(d) => d,
        Err(e) => return format!("    <gone: {:#x}>", e.code().0),
    };
    let mut lines = vec![format!("    id = {id}")];
    if let Ok(state) = unsafe { device.GetState() } {
        lines.push(format!("    state = {}", state_name(state)));
    }
    let Ok(store) = (unsafe { device.OpenPropertyStore(STGM_READ) }) else {
        lines.push("    <properties unreadable>".into());
        return lines.join("\n");
    };
    for (label, fmtid, pid) in ENDPOINT_PROPS {
        let k = key(*fmtid, *pid);
        let Ok(mut value) = (unsafe { store.GetValue(&k) }) else {
            continue;
        };
        let text = unsafe { PropVariantToStringAlloc(&value) }
            .ok()
            .map(|p| unsafe {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            })
            .unwrap_or_default();
        unsafe {
            let _ = PropVariantClear(&mut value);
        }
        if text.is_empty() {
            continue;
        }
        let shown = match (*label, text.parse::<usize>()) {
            ("form factor", Ok(n)) => format!("{n} ({})", FORM_FACTORS.get(n).unwrap_or(&"?")),
            _ => text,
        };
        lines.push(format!("    {label} = {shown}"));
    }
    lines.join("\n")
}

fn log_defaults(devices: &IMMDeviceEnumerator) {
    for flow in [eRender, eCapture] {
        for role in [eConsole, eCommunications] {
            match unsafe { devices.GetDefaultAudioEndpoint(flow, role) } {
                Ok(d) => {
                    let id = device_id(&d);
                    log(
                        "sound",
                        format!(
                            "default {} ({})\n{}",
                            flow_name(flow),
                            role_name(role),
                            describe_endpoint(devices, &id)
                        ),
                    );
                }
                Err(e) => log(
                    "sound",
                    format!(
                        "default {} ({}): none {:#x}",
                        flow_name(flow),
                        role_name(role),
                        e.code().0
                    ),
                ),
            }
        }
    }
}

enum SoundEvent {
    Default(EDataFlow, ERole, String),
    State(String, DEVICE_STATE),
    Added(String),
    Removed(String),
    Property(String, PROPERTYKEY),
}

#[implement(IMMNotificationClient)]
struct SoundEvents {
    tx: Mutex<Sender<SoundEvent>>,
}

impl SoundEvents_Impl {
    fn send(&self, event: SoundEvent) {
        let _ = self.tx.lock().unwrap().send(event);
    }
}

fn wide(id: &PCWSTR) -> String {
    if id.is_null() {
        String::new()
    } else {
        unsafe { id.to_string().unwrap_or_default() }
    }
}

// Callbacks only hand the event over: Windows asks not to block or call back into the
// enumerator from inside them.
impl IMMNotificationClient_Impl for SoundEvents_Impl {
    fn OnDeviceStateChanged(&self, id: &PCWSTR, state: DEVICE_STATE) -> Result<()> {
        self.send(SoundEvent::State(wide(id), state));
        Ok(())
    }
    fn OnDeviceAdded(&self, id: &PCWSTR) -> Result<()> {
        self.send(SoundEvent::Added(wide(id)));
        Ok(())
    }
    fn OnDeviceRemoved(&self, id: &PCWSTR) -> Result<()> {
        self.send(SoundEvent::Removed(wide(id)));
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, id: &PCWSTR) -> Result<()> {
        self.send(SoundEvent::Default(flow, role, wide(id)));
        Ok(())
    }
    fn OnPropertyValueChanged(&self, id: &PCWSTR, key: &PROPERTYKEY) -> Result<()> {
        self.send(SoundEvent::Property(wide(id), *key));
        Ok(())
    }
}

/// Runs on its own thread for the whole probe: the enumerator can't leave the thread it was
/// made on, and the events are read with it.
fn sound() -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let devices: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let all = unsafe { devices.EnumAudioEndpoints(eAll, DEVICE_STATE(DEVICE_STATEMASK_ALL))? };
    let mut shown = 0;
    for i in 0..unsafe { all.GetCount()? } {
        let d = unsafe { all.Item(i)? };
        let state = unsafe { d.GetState()? };
        // Not present = drivers of devices long gone; only clutter.
        if state.0 == 4 {
            continue;
        }
        let id = device_id(&d);
        log(
            "sound",
            format!("endpoint\n{}", describe_endpoint(&devices, &id)),
        );
        shown += 1;
    }
    log(
        "sound",
        format!("{shown} endpoints (not-present ones skipped)"),
    );
    log_defaults(&devices);

    let (tx, rx) = channel();
    let client: IMMNotificationClient = SoundEvents { tx: Mutex::new(tx) }.into();
    unsafe { devices.RegisterEndpointNotificationCallback(&client)? };
    {
        let mut last_property: HashMap<String, Instant> = HashMap::new();
        for event in rx {
            match event {
                SoundEvent::Default(flow, role, id) => log(
                    "sound",
                    format!(
                        "DEFAULT CHANGED {} ({})\n{}",
                        flow_name(flow),
                        role_name(role),
                        describe_endpoint(&devices, &id)
                    ),
                ),
                SoundEvent::State(id, state) => log(
                    "sound",
                    format!(
                        "STATE -> {}\n{}",
                        state_name(state),
                        describe_endpoint(&devices, &id)
                    ),
                ),
                SoundEvent::Added(id) => log(
                    "sound",
                    format!("ADDED\n{}", describe_endpoint(&devices, &id)),
                ),
                SoundEvent::Removed(id) => log("sound", format!("REMOVED {id}")),
                SoundEvent::Property(id, k) => {
                    // Windows sends a burst of these on every change; one line per endpoint
                    // and key a second is enough.
                    let tag = format!("{id} {:?} {}", k.fmtid, k.pid);
                    let now = Instant::now();
                    if last_property
                        .get(&tag)
                        .is_some_and(|t| now.duration_since(*t) < Duration::from_secs(1))
                    {
                        continue;
                    }
                    last_property.insert(tag, now);
                    log(
                        "sound",
                        format!("property {:?} {} changed on {id}", k.fmtid, k.pid),
                    );
                }
            }
        }
    }
    drop(client);
    Ok(())
}

const MAKERS: &[(u16, &str)] = &[
    (0x004c, "Apple"),
    (0x0075, "Samsung"),
    (0x0006, "Microsoft"),
    (0x012d, "Sony"),
    (0x038f, "Xiaomi"),
    (0x027d, "Huawei"),
    (0x0057, "Harman"),
    (0x0087, "Garmin"),
    (0x00e0, "Google"),
    (0x0171, "Amazon"),
    (0x0310, "Jabra (GN)"),
    (0x009e, "Bose"),
];

/// Apple "proximity pairing" (type 0x07): what AirPods and Beats send while their case is open
/// or they are out. Layout from public reverse engineering; to be checked against the log.
fn apple_pairing(data: &[u8]) -> Option<String> {
    if data.len() < 9 || data[0] != 0x07 {
        return None;
    }
    let model = u16::from_be_bytes([data[3], data[4]]);
    let level = |n: u8| {
        if n <= 10 {
            format!("{}%", n * 10)
        } else {
            "?".into()
        }
    };
    let pods = data[6];
    let case_byte = data[7];
    Some(format!(
        "apple pairing: model 0x{model:04x} status 0x{:02x} pods {}/{} case {} charging-bits 0x{:x} lid 0x{:02x}",
        data[5],
        level(pods >> 4),
        level(pods & 0x0f),
        level(case_byte & 0x0f),
        case_byte >> 4,
        data[8]
    ))
}

/// (address, company) → when and what was logged last.
type LastAds = HashMap<(u64, u16), (Instant, Vec<u8>)>;

fn start_advertisements() -> Result<BluetoothLEAdvertisementWatcher> {
    let watcher = BluetoothLEAdvertisementWatcher::new()?;
    watcher.SetScanningMode(BluetoothLEScanningMode::Active)?;
    let seen: &'static Mutex<LastAds> = Box::leak(Box::new(Mutex::new(HashMap::new())));
    let skipped: &'static Mutex<HashMap<u16, u32>> =
        Box::leak(Box::new(Mutex::new(HashMap::new())));
    watcher.Received(&TypedEventHandler::<
        BluetoothLEAdvertisementWatcher,
        BluetoothLEAdvertisementReceivedEventArgs,
    >::new(move |_, args| {
        let Ok(args) = args.ok() else { return Ok(()) };
        let address = args.BluetoothAddress()?;
        let rssi = args.RawSignalStrengthInDBm()?;
        let ad = args.Advertisement()?;
        let name = ad.LocalName().map(|n| n.to_string()).unwrap_or_default();
        let mut found: Vec<(u16, Vec<u8>)> = Vec::new();
        let makers = ad.ManufacturerData()?;
        for i in 0..makers.Size()? {
            let m = makers.GetAt(i)?;
            found.push((m.CompanyId()?, buffer_bytes(&m.Data()?)?));
        }
        // Google Fast Pair is service data (0x16) for UUID 0xFE2C, not maker data.
        let sections = ad.DataSections()?;
        for i in 0..sections.Size()? {
            let s = sections.GetAt(i)?;
            if s.DataType()? == 0x16 {
                let bytes = buffer_bytes(&s.Data()?)?;
                if bytes.starts_with(&[0x2c, 0xfe]) {
                    found.push((0xfe2c, bytes));
                }
            }
        }
        for (company, data) in found {
            let maker = MAKERS
                .iter()
                .find(|(id, _)| *id == company)
                .map(|(_, n)| *n)
                .or((company == 0xfe2c).then_some("Google Fast Pair"));
            let Some(maker) = maker else { continue };
            let decoded = if company == 0x004c { apple_pairing(&data) } else { None };
            // iPhones, Macs and Samsung phones nearby advertise all the time; Apple's other
            // types are counted, not logged.
            if company == 0x004c && decoded.is_none() {
                *skipped.lock().unwrap().entry(company).or_default() += 1;
                continue;
            }
            {
                let mut seen = seen.lock().unwrap();
                let now = Instant::now();
                let tag = (address, company);
                if let Some((when, last)) = seen.get(&tag)
                    && (*last == data || now.duration_since(*when) < Duration::from_secs(5)) {
                        continue;
                    }
                seen.insert(tag, (now, data.clone()));
            }
            let mut line = format!(
                "{maker} 0x{company:04x} from {address:012x} rssi {rssi} name \"{name}\" data[{}] {}",
                data.len(),
                hex(&data)
            );
            if let Some(d) = decoded {
                line.push_str(&format!("\n    {d}"));
            }
            log("ble-ad", line);
        }
        Ok(())
    }))?;
    watcher.Start()?;
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(60));
            let mut skipped = skipped.lock().unwrap();
            if let Some(n) = skipped.remove(&0x004c) {
                log(
                    "ble-ad",
                    format!("{n} other Apple advertisements in the last minute (phones, Macs)"),
                );
            }
        }
    });
    Ok(watcher)
}

fn describe_controller(c: &RawGameController) -> String {
    let mut lines = vec![
        format!(
            "    name = \"{}\"",
            c.DisplayName().map(|n| n.to_string()).unwrap_or_default()
        ),
        format!(
            "    vendor 0x{:04x} product 0x{:04x} wireless {}",
            c.HardwareVendorId().unwrap_or(0),
            c.HardwareProductId().unwrap_or(0),
            c.IsWireless().unwrap_or(false)
        ),
        format!(
            "    id = {}",
            c.NonRoamableId().map(|n| n.to_string()).unwrap_or_default()
        ),
    ];
    match c.TryGetBatteryReport() {
        Ok(report) => {
            let remaining = report
                .RemainingCapacityInMilliwattHours()
                .and_then(|v| v.Value())
                .ok();
            let full = report
                .FullChargeCapacityInMilliwattHours()
                .and_then(|v| v.Value())
                .ok();
            lines.push(format!(
                "    battery: {remaining:?} of {full:?} mWh, status {}",
                report.Status().map(|s| s.0).unwrap_or(-1)
            ));
        }
        Err(e) => lines.push(format!("    battery: none ({:#x})", e.code().0)),
    }
    lines.join("\n")
}

fn start_gamepads() -> Result<()> {
    let known = RawGameController::RawGameControllers()?;
    for i in 0..known.Size()? {
        log(
            "gamepad",
            format!("present\n{}", describe_controller(&known.GetAt(i)?)),
        );
    }
    RawGameController::RawGameControllerAdded(&EventHandler::<RawGameController>::new(|_, c| {
        if let Ok(c) = c.ok() {
            log("gamepad", format!("ADDED\n{}", describe_controller(c)));
        }
        Ok(())
    }))?;
    RawGameController::RawGameControllerRemoved(&EventHandler::<RawGameController>::new(
        |_, c| {
            if let Ok(c) = c.ok() {
                log(
                    "gamepad",
                    format!(
                        "REMOVED \"{}\"",
                        c.DisplayName().map(|n| n.to_string()).unwrap_or_default()
                    ),
                );
            }
            Ok(())
        },
    ))?;
    // Windows sends no event when a gamepad's battery changes.
    std::thread::spawn(|| {
        let mut last: HashMap<String, String> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_secs(30));
            let Ok(all) = RawGameController::RawGameControllers() else {
                continue;
            };
            for i in 0..all.Size().unwrap_or(0) {
                let Ok(c) = all.GetAt(i) else { continue };
                let id = c.NonRoamableId().map(|n| n.to_string()).unwrap_or_default();
                let text = describe_controller(&c);
                if last.get(&id) != Some(&text) {
                    log("gamepad", format!("now\n{text}"));
                    last.insert(id, text);
                }
            }
        }
    });
    Ok(())
}

fn main() {
    let folder = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let path = folder.join("device_probe.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    let _ = LOG.set(Mutex::new(Log {
        file,
        started: Instant::now(),
    }));
    log(
        "probe",
        format!("==== started {} -> {}", clock(), path.display()),
    );
    log(
        "probe",
        "type a note and Enter to mark the log; q + Enter ends",
    );

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let (details_tx, details_rx) = channel::<String>();
    std::thread::spawn(move || {
        // A device reports "connected" several times in a row; read its details once a minute.
        let mut done: HashMap<String, Instant> = HashMap::new();
        let mut busy = HashSet::new();
        for id in details_rx {
            let now = Instant::now();
            if done
                .get(&id)
                .is_some_and(|t| now.duration_since(*t) < Duration::from_secs(60))
                || !busy.insert(id.clone())
            {
                continue;
            }
            done.insert(id.clone(), now);
            let worker_id = id.clone();
            std::thread::spawn(move || bluetooth_details(worker_id));
            busy.remove(&id);
        }
    });

    // Paired Bluetooth devices, classic (e0cbf06c…) and LE (bb7bb05e…).
    watch(
        "bt",
        DeviceInformationKind::AssociationEndpoint,
        "(System.Devices.Aep.ProtocolId:=\"{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}\" OR \
         System.Devices.Aep.ProtocolId:=\"{bb7bb05e-5972-42b5-94fc-76eaa7084d49}\") AND \
         System.Devices.Aep.IsPaired:=System.StructuredQueryType.Boolean#True",
        AEP_PROPS,
        every_device,
        Some(details_tx),
    );
    watch(
        "container",
        DeviceInformationKind::DeviceContainer,
        "",
        CONTAINER_PROPS,
        every_device,
        None,
    );
    watch(
        "pnp",
        DeviceInformationKind::Device,
        "",
        DEVICE_PROPS,
        device_worth_logging,
        None,
    );

    std::thread::spawn(|| {
        if let Err(e) = sound() {
            log_err("sound", "start", e);
        }
    });
    let _ads = match start_advertisements() {
        Ok(w) => Some(w),
        Err(e) => {
            log_err("ble-ad", "start", e);
            None
        }
    };
    if let Err(e) = start_gamepads() {
        log_err("gamepad", "start", e);
    }

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.eq_ignore_ascii_case("q") {
            break;
        }
        if !line.is_empty() {
            log("NOTE", format!(">>>>>>>> {line} ({})", clock()));
        }
    }
    log("probe", format!("==== ended {}", clock()));
}
