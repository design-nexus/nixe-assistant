//! Bluetooth and batteries, read and changed over D-Bus (BlueZ and UPower on the system bus)
//! through `busctl --json`, so replies are typed data rather than parsed text. Scanning and
//! pairing use `bluetoothctl`: BlueZ ends a discovery when the client that started it goes
//! away, and pairing needs an agent.
//!
//! Devices are only ever found among the ones BlueZ already knows; the object paths the
//! calls use come from BlueZ, never from the request.

use crate::system::{run, settle};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub const TOOLS: &[(&str, &str, &str)] = &[
    (
        "bluetooth.status",
        "Bluetooth: whether it's on, and the known devices (name, address, paired, connected, battery).",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
    (
        "bluetooth.power",
        "Turn Bluetooth on or off.",
        r#"{"type":"object","properties":{"on":{"type":"boolean"}},"required":["on"],"additionalProperties":false}"#,
    ),
    (
        "bluetooth.connect",
        "Connect a paired Bluetooth device, by name or address.",
        r#"{"type":"object","properties":{"device":{"type":"string","maxLength":128}},"required":["device"],"additionalProperties":false}"#,
    ),
    (
        "bluetooth.disconnect",
        "Disconnect a Bluetooth device, by name or address.",
        r#"{"type":"object","properties":{"device":{"type":"string","maxLength":128}},"required":["device"],"additionalProperties":false}"#,
    ),
    (
        "bluetooth.scan",
        "Look for nearby Bluetooth devices for a few seconds (default 8, at most 20); lists the ones not paired.",
        r#"{"type":"object","properties":{"seconds":{"type":"integer","minimum":3,"maximum":20}},"additionalProperties":false}"#,
    ),
    (
        "bluetooth.pair",
        "Pair, trust and connect a device found by bluetooth.scan, by name or address. Works for devices that need no PIN (headphones, speakers, mice).",
        r#"{"type":"object","properties":{"device":{"type":"string","maxLength":128}},"required":["device"],"additionalProperties":false}"#,
    ),
    (
        "bluetooth.forget",
        "Remove a paired Bluetooth device, by name or address.",
        r#"{"type":"object","properties":{"device":{"type":"string","maxLength":128}},"required":["device"],"additionalProperties":false}"#,
    ),
    (
        "battery.status",
        "Batteries and power: the laptop battery (percent, charging, time left, health), whether it's plugged in, and the batteries of mice, keyboards and headsets.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    ),
];

pub async fn call(name: &str, args: &Value) -> Option<Result<Value>> {
    let device = || args["device"].as_str().unwrap_or("").to_string();
    Some(match name {
        "bluetooth.status" => async {
            let objects = bluez().await?;
            Ok(json!({"adapter": adapter(&objects), "devices": devices(&objects)}))
        }
        .await,
        "bluetooth.power" => async {
            let on = args["on"].as_bool().context("on must be true or false")?;
            let path = adapter_path(&bluez().await?)?;
            run("busctl", &["--system", "set-property", "org.bluez", &path, "org.bluez.Adapter1", "Powered", "b", if on { "true" } else { "false" }]).await?;
            let verified = settle(3000, || async { bluez().await.ok().is_some_and(|o| adapter(&o)["powered"] == on) }).await;
            Ok(json!({"powered": on, "verified": verified}))
        }
        .await,
        "bluetooth.connect" | "bluetooth.disconnect" => async {
            let connect = name == "bluetooth.connect";
            let objects = bluez().await?;
            ensure!(adapter(&objects)["powered"] == true, "Bluetooth is off; turn it on first");
            let d = find(&devices(&objects), &device(), connect)?;
            let path = d["path"].as_str().unwrap_or("").to_string();
            let method = if connect { "Connect" } else { "Disconnect" };
            run("busctl", &["--system", "--timeout=30", "call", "org.bluez", &path, "org.bluez.Device1", method]).await?;
            let verified = settle(5000, || async { connected(&path).await == Some(connect) }).await;
            Ok(json!({"device": d["name"], "address": d["address"], "connected": connect, "verified": verified}))
        }
        .await,
        "bluetooth.scan" => async {
            let seconds = args["seconds"].as_u64().unwrap_or(8).clamp(3, 20);
            ensure!(adapter(&bluez().await?)["powered"] == true, "Bluetooth is off; turn it on first");
            // bluetoothctl exits after the timeout; discovery ends with it.
            let _ = tokio::time::timeout(
                Duration::from_secs(seconds + 5),
                tokio::process::Command::new("bluetoothctl").args(["--timeout", &seconds.to_string(), "scan", "on"]).kill_on_drop(true).output(),
            )
            .await;
            let found: Vec<Value> = devices(&bluez().await?).into_iter().filter(|d| d["paired"] != true).collect();
            Ok(json!({"seconds": seconds, "found": found}))
        }
        .await,
        "bluetooth.pair" => async {
            let objects = bluez().await?;
            ensure!(adapter(&objects)["powered"] == true, "Bluetooth is off; turn it on first");
            let d = find(&devices(&objects), &device(), false)?;
            ensure!(d["paired"] != true, "{} is already paired", d["name"].as_str().unwrap_or("it"));
            let (path, address) = (d["path"].as_str().unwrap_or("").to_string(), d["address"].as_str().unwrap_or("").to_string());
            ensure!(valid_address(&address), "invalid device address");
            // An agent that can't show or type a PIN: devices that need one are refused.
            let out = run("bluetoothctl", &["--timeout", "30", "--agent", "NoInputNoOutput", "pair", &address]).await.unwrap_or_default();
            let paired = settle(5000, || async { property(&path, "Paired").await == Some(json!(true)) }).await;
            ensure!(paired, "pairing didn't complete{}", if out.to_lowercase().contains("pin") || out.to_lowercase().contains("passkey") { ": this device needs a PIN; pair it in the Bluetooth settings" } else { "" });
            run("busctl", &["--system", "set-property", "org.bluez", &path, "org.bluez.Device1", "Trusted", "b", "true"]).await?;
            let _ = run("busctl", &["--system", "--timeout=30", "call", "org.bluez", &path, "org.bluez.Device1", "Connect"]).await;
            let connected = settle(5000, || async { connected(&path).await == Some(true) }).await;
            Ok(json!({"device": d["name"], "address": address, "paired": true, "trusted": true, "connected": connected, "verified": true}))
        }
        .await,
        "bluetooth.forget" => async {
            let objects = bluez().await?;
            let d = find(&devices(&objects), &device(), true)?;
            let (adapter, path) = (adapter_path(&objects)?, d["path"].as_str().unwrap_or("").to_string());
            run("busctl", &["--system", "call", "org.bluez", &adapter, "org.bluez.Adapter1", "RemoveDevice", "o", &path]).await?;
            let verified = settle(3000, || async { bluez().await.ok().is_some_and(|o| !devices(&o).iter().any(|x| x["path"] == path.as_str())) }).await;
            Ok(json!({"device": d["name"], "address": d["address"], "removed": true, "verified": verified}))
        }
        .await,
        "battery.status" => battery().await,
        _ => return None,
    })
}

/// BlueZ's objects: `GetManagedObjects`' reply, path → interface → property → `{type, data}`.
async fn bluez() -> Result<Value> {
    let out = run("busctl", &["--system", "--json=short", "call", "org.bluez", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects"])
        .await
        .context("Bluetooth isn't available (is bluetooth.service running?)")?;
    let reply: Value = serde_json::from_str(&out)?;
    Ok(reply["data"][0].clone())
}

async fn property(path: &str, name: &str) -> Option<Value> {
    let out = run("busctl", &["--system", "--json=short", "get-property", "org.bluez", path, "org.bluez.Device1", name]).await.ok()?;
    serde_json::from_str::<Value>(&out).ok().map(|v| v["data"].clone())
}

async fn connected(path: &str) -> Option<bool> {
    property(path, "Connected").await.and_then(|v| v.as_bool())
}

fn prop<'a>(props: &'a Value, name: &str) -> &'a Value {
    &props[name]["data"]
}

/// The first adapter: address, name, powered, discovering.
fn adapter(objects: &Value) -> Value {
    let Some((path, props)) = objects.as_object().and_then(|o| o.iter().find_map(|(p, i)| i.get("org.bluez.Adapter1").map(|a| (p, a)))) else {
        return json!({"present": false, "powered": false});
    };
    json!({"present": true, "path": path, "name": prop(props, "Alias"), "address": prop(props, "Address"), "powered": prop(props, "Powered").as_bool().unwrap_or(false), "discovering": prop(props, "Discovering").as_bool().unwrap_or(false)})
}

fn adapter_path(objects: &Value) -> Result<String> {
    adapter(objects)["path"].as_str().map(String::from).context("no Bluetooth adapter")
}

/// Every device BlueZ knows, paired first, then by name: name, address, kind, paired,
/// connected, trusted, battery percent (when the device reports it), signal (while scanning).
fn devices(objects: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = objects
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(path, ifaces)| {
                    let d = ifaces.get("org.bluez.Device1")?;
                    let address = prop(d, "Address").as_str()?.to_string();
                    let name = prop(d, "Alias").as_str().or_else(|| prop(d, "Name").as_str()).unwrap_or(&address).to_string();
                    Some(json!({
                        "path": path,
                        "name": name,
                        "address": address,
                        "kind": prop(d, "Icon").as_str().unwrap_or(""),
                        "paired": prop(d, "Paired").as_bool().unwrap_or(false),
                        "connected": prop(d, "Connected").as_bool().unwrap_or(false),
                        "trusted": prop(d, "Trusted").as_bool().unwrap_or(false),
                        "battery_percent": ifaces.get("org.bluez.Battery1").and_then(|b| prop(b, "Percentage").as_u64()),
                        "signal_dbm": prop(d, "RSSI").as_i64(),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|d| (d["paired"] != true, d["name"].as_str().unwrap_or("").to_lowercase()));
    out
}

fn valid_address(s: &str) -> bool {
    s.len() == 17 && s.split(':').count() == 6 && s.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The one device a request names: its address, or its name exactly, then a name that starts
/// with or contains the words. `paired_only` leaves out devices only seen in a scan.
fn find(devices: &[Value], wanted: &str, paired_only: bool) -> Result<Value> {
    let wanted = wanted.trim();
    ensure!(!wanted.is_empty(), "name the device");
    let pool: Vec<&Value> = devices.iter().filter(|d| !paired_only || d["paired"] == true).collect();
    if valid_address(wanted) {
        return pool.iter().find(|d| d["address"].as_str().is_some_and(|a| a.eq_ignore_ascii_case(wanted))).map(|d| (*d).clone()).context("no known device with that address");
    }
    let w = wanted.to_lowercase();
    let name = |d: &&Value| d["name"].as_str().unwrap_or("").to_lowercase();
    for test in [
        &(|n: &str| n == w) as &dyn Fn(&str) -> bool,
        &(|n: &str| n.starts_with(&w)),
        &(|n: &str| n.contains(&w)),
    ] {
        let hits: Vec<&&Value> = pool.iter().filter(|d| test(&name(d))).collect();
        match hits.len() {
            0 => continue,
            1 => return Ok((*hits[0]).clone()),
            _ => anyhow::bail!("several devices match: {}", hits.iter().map(|d| d["name"].as_str().unwrap_or("")).collect::<Vec<_>>().join(", ")),
        }
    }
    anyhow::bail!(
        "no {}device called {wanted}{}",
        if paired_only { "paired " } else { "" },
        if paired_only { "" } else { "; run bluetooth.scan first" }
    )
}

const UPOWER: &str = "org.freedesktop.UPower";

async fn battery() -> Result<Value> {
    let out = run("busctl", &["--system", "--json=short", "call", UPOWER, "/org/freedesktop/UPower", UPOWER, "EnumerateDevices"])
        .await
        .context("UPower isn't available")?;
    let reply: Value = serde_json::from_str(&out)?;
    let mut devices = Vec::new();
    for path in reply["data"][0].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let props = run("busctl", &["--system", "--json=short", "call", UPOWER, path, "org.freedesktop.DBus.Properties", "GetAll", "s", "org.freedesktop.UPower.Device"]).await?;
        let props: Value = serde_json::from_str(&props)?;
        devices.push(props["data"][0].clone());
    }
    Ok(power_summary(&devices))
}

fn kind(code: u64) -> &'static str {
    match code {
        1 => "line power",
        2 => "battery",
        3 => "ups",
        5 => "mouse",
        6 => "keyboard",
        8 => "phone",
        10 => "tablet",
        12 => "game controller",
        13 => "pen",
        14 => "touchpad",
        17 => "headset",
        18 => "speakers",
        19 => "headphones",
        _ => "device",
    }
}

fn state(code: u64) -> &'static str {
    match code {
        1 => "charging",
        2 => "discharging",
        3 => "empty",
        4 => "full",
        5 => "not charging",
        6 => "waiting to discharge",
        _ => "unknown",
    }
}

/// UPower's devices (each `GetAll`'s properties) as: plugged in, the laptop's batteries, and
/// other devices' batteries.
fn power_summary(devices: &[Value]) -> Value {
    let num = |d: &Value, k: &str| prop(d, k).as_f64().unwrap_or(0.0);
    let int = |d: &Value, k: &str| prop(d, k).as_u64().unwrap_or(0);
    let plugged_in = devices.iter().any(|d| int(d, "Type") == 1 && prop(d, "Online") == true);
    let mut batteries = Vec::new();
    let mut others = Vec::new();
    for d in devices {
        let t = int(d, "Type");
        if t == 1 || (t == 2 && prop(d, "IsPresent") == false) {
            continue;
        }
        let minutes = |k: &str| (int(d, k) > 0).then(|| int(d, k) / 60);
        let warning = match int(d, "WarningLevel") {
            3 => Some("low"),
            4 => Some("critical"),
            5 => Some("shutting down soon"),
            _ => None,
        };
        let name: Vec<&str> = ["Vendor", "Model"].iter().filter_map(|k| prop(d, k).as_str()).filter(|s| !s.is_empty()).collect();
        let entry = json!({
            "kind": kind(t),
            "name": name.join(" "),
            "percent": num(d, "Percentage").round() as u64,
            "state": state(int(d, "State")),
            "minutes_to_empty": minutes("TimeToEmpty"),
            "minutes_to_full": minutes("TimeToFull"),
            "watts": (num(d, "EnergyRate") > 0.0).then(|| (num(d, "EnergyRate") * 10.0).round() / 10.0),
            "health_percent": (num(d, "Capacity") > 0.0).then(|| num(d, "Capacity").round() as u64),
            "warning": warning,
        });
        if t == 2 && prop(d, "PowerSupply") == true {
            batteries.push(entry);
        } else {
            others.push(entry);
        }
    }
    json!({"plugged_in": plugged_in, "batteries": batteries, "devices": others})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str, v: Value) -> Value {
        json!({"type": t, "data": v})
    }

    fn objects() -> Value {
        json!({
            "/org/bluez/hci0": {"org.bluez.Adapter1": {"Address": p("s", json!("28:95:29:8E:0F:66")), "Alias": p("s", json!("Hades")), "Powered": p("b", json!(true)), "Discovering": p("b", json!(false))}},
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_01": {
                "org.bluez.Device1": {"Address": p("s", json!("AA:BB:CC:DD:EE:01")), "Alias": p("s", json!("WH-1000XM5")), "Icon": p("s", json!("audio-headset")), "Paired": p("b", json!(true)), "Connected": p("b", json!(true)), "Trusted": p("b", json!(true))},
                "org.bluez.Battery1": {"Percentage": p("y", json!(85))}
            },
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_02": {"org.bluez.Device1": {"Address": p("s", json!("AA:BB:CC:DD:EE:02")), "Alias": p("s", json!("MX Master 3")), "Paired": p("b", json!(true)), "Connected": p("b", json!(false))}},
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_03": {"org.bluez.Device1": {"Address": p("s", json!("AA:BB:CC:DD:EE:03")), "Name": p("s", json!("JBL Flip 6")), "Paired": p("b", json!(false)), "RSSI": p("n", json!(-61))}},
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_04": {"org.bluez.Device1": {"Address": p("s", json!("AA:BB:CC:DD:EE:04")), "Alias": p("s", json!("MX Keys")), "Paired": p("b", json!(true))}}
        })
    }

    #[test]
    fn reads_the_adapter_and_devices() {
        let o = objects();
        let a = adapter(&o);
        assert_eq!((a["powered"].clone(), a["name"].clone(), a["path"].clone()), (json!(true), json!("Hades"), json!("/org/bluez/hci0")));
        let d = devices(&o);
        let names: Vec<&str> = d.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["MX Keys", "MX Master 3", "WH-1000XM5", "JBL Flip 6"], "paired first, then by name");
        let headset = &d[2];
        assert_eq!((headset["battery_percent"].clone(), headset["connected"].clone(), headset["kind"].clone()), (json!(85), json!(true), json!("audio-headset")));
        assert_eq!(d[3]["signal_dbm"], -61);
        assert_eq!(adapter(&json!({}))["present"], false);
    }

    #[test]
    fn finds_one_device_by_address_or_name() {
        let d = devices(&objects());
        assert_eq!(find(&d, "aa:bb:cc:dd:ee:01", true).unwrap()["name"], "WH-1000XM5");
        assert_eq!(find(&d, "wh-1000xm5", true).unwrap()["address"], "AA:BB:CC:DD:EE:01", "exact name, any case");
        assert_eq!(find(&d, "MX Master", true).unwrap()["name"], "MX Master 3", "starts with");
        assert!(find(&d, "MX", true).unwrap_err().to_string().contains("several devices match"));
        assert!(find(&d, "JBL", true).is_err(), "not paired");
        assert_eq!(find(&d, "JBL", false).unwrap()["name"], "JBL Flip 6", "found in a scan");
        assert!(find(&d, "", false).is_err());
        assert!(find(&d, "AirPods", false).unwrap_err().to_string().contains("bluetooth.scan"));
    }

    #[test]
    fn addresses_are_checked() {
        assert!(valid_address("AA:BB:CC:DD:EE:01"));
        for bad in ["AA:BB:CC:DD:EE", "AA:BB:CC:DD:EE:0G", "AA-BB-CC-DD-EE-01", "AA:BB:CC:DD:EE:01; rm"] {
            assert!(!valid_address(bad), "{bad}");
        }
    }

    #[test]
    fn summarizes_power() {
        let battery = json!({"Type": p("u", json!(2)), "Vendor": p("s", json!("ASUS")), "Model": p("s", json!("A32-K55")), "PowerSupply": p("b", json!(true)), "IsPresent": p("b", json!(true)),
            "Percentage": p("d", json!(73.4)), "State": p("u", json!(2)), "TimeToEmpty": p("x", json!(7260)), "TimeToFull": p("x", json!(0)), "EnergyRate": p("d", json!(14.27)), "Capacity": p("d", json!(98.39)), "WarningLevel": p("u", json!(1))});
        let ac = json!({"Type": p("u", json!(1)), "Online": p("b", json!(false)), "PowerSupply": p("b", json!(true))});
        let mouse = json!({"Type": p("u", json!(5)), "Model": p("s", json!("MX Master 3")), "PowerSupply": p("b", json!(false)), "IsPresent": p("b", json!(true)), "Percentage": p("d", json!(12.0)), "State": p("u", json!(2)), "WarningLevel": p("u", json!(3))});
        let s = power_summary(&[battery, ac, mouse]);
        assert_eq!(s["plugged_in"], false);
        let b = &s["batteries"][0];
        assert_eq!((b["name"].clone(), b["percent"].clone(), b["state"].clone(), b["minutes_to_empty"].clone(), b["watts"].clone(), b["health_percent"].clone()), (json!("ASUS A32-K55"), json!(73), json!("discharging"), json!(121), json!(14.3), json!(98)));
        assert!(b["minutes_to_full"].is_null());
        let m = &s["devices"][0];
        assert_eq!((m["kind"].clone(), m["percent"].clone(), m["warning"].clone()), (json!("mouse"), json!(12), json!("low")));
        // Plugged in, no battery present (a desktop).
        let ac_on = json!({"Type": p("u", json!(1)), "Online": p("b", json!(true))});
        let empty_bay = json!({"Type": p("u", json!(2)), "IsPresent": p("b", json!(false)), "PowerSupply": p("b", json!(true))});
        let s = power_summary(&[ac_on, empty_bay]);
        assert_eq!((s["plugged_in"].clone(), s["batteries"].as_array().unwrap().len()), (json!(true), 0));
    }
}
