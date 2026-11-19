//! Two device watchers: the paired Bluetooth devices (classic and LE) with their link, kind and
//! container, and the Bluetooth device nodes that carry a battery. Windows sends every change by
//! itself; the callbacks only hand the properties to the watch's thread.

use super::watch::Raw;
use std::{collections::HashMap, sync::mpsc::Sender};
use windows::{
    Devices::Enumeration::{
        DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher,
    },
    Foundation::{IPropertyValue, PropertyType, TypedEventHandler},
    core::{HSTRING, IInspectable, Interface, Result},
};
use windows_collections::{IIterable, IMapView};

/// The battery Windows' own Bluetooth page shows (`DEVPKEY_Bluetooth_Battery`), on the device's
/// hands-free node for headsets, on the device node for LE mice and keyboards.
pub(super) const BATTERY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";
pub(super) const NAME: &str = "System.ItemNameDisplay";
pub(super) const CONNECTED: &str = "System.Devices.Aep.IsConnected";
pub(super) const CATEGORY: &str = "System.Devices.Aep.Category";
pub(super) const MAJOR: &str = "System.Devices.Aep.Bluetooth.Cod.Major";
pub(super) const MINOR: &str = "System.Devices.Aep.Bluetooth.Cod.Minor";
pub(super) const APPEARANCE: &str = "System.Devices.Aep.Bluetooth.Le.Appearance";
pub(super) const PAIRED_CONTAINER: &str = "System.Devices.Aep.ContainerId";
pub(super) const NODE_CONTAINER: &str = "System.Devices.ContainerId";

/// Paired devices only: classic Bluetooth (e0cbf06c…) and LE (bb7bb05e…).
const PAIRED: &str = "(System.Devices.Aep.ProtocolId:=\"{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}\" OR \
     System.Devices.Aep.ProtocolId:=\"{bb7bb05e-5972-42b5-94fc-76eaa7084d49}\") AND \
     System.Devices.Aep.IsPaired:=System.StructuredQueryType.Boolean#True";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Paired,
    Nodes,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Flag(bool),
    Number(u32),
    Text(String),
    Texts(Vec<String>),
}

pub(super) type Props = HashMap<String, Value>;

pub(super) fn watch(source: Source, tx: &Sender<Raw>) -> Result<DeviceWatcher> {
    let (aqs, names, kind): (&str, &[&str], _) = match source {
        Source::Paired => (
            PAIRED,
            &[
                NAME,
                CONNECTED,
                CATEGORY,
                MAJOR,
                MINOR,
                APPEARANCE,
                PAIRED_CONTAINER,
            ],
            DeviceInformationKind::AssociationEndpoint,
        ),
        // Every device of the computer: the filter is by the id, in the callback. Windows' query
        // language has no reliable "starts with" for the instance id.
        Source::Nodes => (
            "",
            &[NODE_CONTAINER, BATTERY],
            DeviceInformationKind::Device,
        ),
    };
    let names: IIterable<HSTRING> = names
        .iter()
        .map(|n| HSTRING::from(*n))
        .collect::<Vec<_>>()
        .into();
    let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(aqs),
        &names,
        kind,
    )?;
    let keep = move |id: &str| source == Source::Paired || is_bluetooth_node(id);

    let sent = tx.clone();
    watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
        move |_, info| {
            if let Ok(info) = info.ok() {
                let id = info.Id()?.to_string();
                if keep(&id) {
                    let mut props = read(&info.Properties()?);
                    props.insert(NAME.into(), Value::Text(info.Name()?.to_string()));
                    let _ = sent.send(Raw::Changed { source, id, props });
                }
            }
            Ok(())
        },
    ))?;
    let sent = tx.clone();
    watcher.Updated(
        &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, update| {
            if let Ok(update) = update.ok() {
                let id = update.Id()?.to_string();
                if keep(&id) {
                    let props = read(&update.Properties()?);
                    let _ = sent.send(Raw::Changed { source, id, props });
                }
            }
            Ok(())
        }),
    )?;
    let sent = tx.clone();
    watcher.Removed(
        &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, update| {
            if let Ok(update) = update.ok() {
                let id = update.Id()?.to_string();
                if keep(&id) {
                    let _ = sent.send(Raw::Gone { source, id });
                }
            }
            Ok(())
        }),
    )?;
    let sent = tx.clone();
    watcher.EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(
        move |_, _| {
            let _ = sent.send(Raw::Listed(source));
            Ok(())
        },
    ))?;
    watcher.Start()?;
    Ok(watcher)
}

/// The battery sits on Bluetooth nodes (`BTHENUM\…`, `BTHLE\…`, `BTHHFENUM\…`).
fn is_bluetooth_node(id: &str) -> bool {
    id.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("BTH"))
}

fn read(props: &IMapView<HSTRING, IInspectable>) -> Props {
    let mut out = Props::new();
    let Ok(it) = props.First() else {
        return out;
    };
    while it.HasCurrent().unwrap_or(false) {
        if let Ok(pair) = it.Current()
            && let (Ok(key), Ok(value)) = (pair.Key(), pair.Value())
            && let Some(value) = value_of(&value)
        {
            out.insert(key.to_string(), value);
        }
        if !it.MoveNext().unwrap_or(false) {
            break;
        }
    }
    out
}

fn value_of(value: &IInspectable) -> Option<Value> {
    let v = value.cast::<IPropertyValue>().ok()?;
    let value = match v.Type().ok()? {
        PropertyType::Boolean => Value::Flag(v.GetBoolean().ok()?),
        PropertyType::UInt8 => Value::Number(v.GetUInt8().ok()?.into()),
        PropertyType::UInt16 => Value::Number(v.GetUInt16().ok()?.into()),
        PropertyType::UInt32 => Value::Number(v.GetUInt32().ok()?),
        PropertyType::Int32 => Value::Number(u32::try_from(v.GetInt32().ok()?).ok()?),
        PropertyType::String => Value::Text(v.GetString().ok()?.to_string()),
        PropertyType::Guid => Value::Text(format!("{:?}", v.GetGuid().ok()?)),
        PropertyType::StringArray => {
            let mut array = windows_core::Array::<HSTRING>::new();
            v.GetStringArray(&mut array).ok()?;
            Value::Texts(array.iter().map(|s| s.to_string()).collect())
        }
        _ => return None,
    };
    Some(value)
}
