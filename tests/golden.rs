//! Baseline wire and storage snapshots, with the management address deliberately left unspecified.
mod common;

use common::{device, ppp_prefix, profile, rpc, spv};
use std::{fs, path::Path};
use yettel_cwmp::{
    cwmp::{model::Assignments, rpc::Cpe, soap},
    domain::{export::Export, secret::Secret},
    store::Store,
};

// A broken synthetic fixture must fail this test immediately.
#[allow(clippy::unwrap_used)]
fn snapshot(name: &str, bytes: &[u8]) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(name);
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        fs::write(&path, bytes).unwrap();
    }
    assert_eq!(bytes, fs::read(&path).unwrap(), "snapshot {name} changed");
}

// A broken synthetic fixture must fail this test immediately.
#[allow(clippy::unwrap_used)]
fn request(name: &str, fields: &str) -> Vec<u8> {
    let mut cpe = Cpe::new(device().model(&profile()).unwrap());
    cpe.handle(&rpc(name, fields)).reply.unwrap()
}

#[test]
fn rpc_wire_snapshots() {
    let cases: [(&str, &str, &str); 12] = [
        ("get_rpc_methods.xml", "GetRPCMethods", ""),
        (
            "get_names_next.xml",
            "GetParameterNames",
            "<ParameterPath>InternetGatewayDevice.DeviceInfo.</ParameterPath><NextLevel>1</NextLevel>",
        ),
        (
            "get_names_all.xml",
            "GetParameterNames",
            "<ParameterPath>InternetGatewayDevice.DeviceInfo.</ParameterPath><NextLevel>0</NextLevel>",
        ),
        (
            "get_names_root.xml",
            "GetParameterNames",
            "<ParameterPath></ParameterPath><NextLevel>1</NextLevel>",
        ),
        (
            "get_value.xml",
            "GetParameterValues",
            "<ParameterNames><string>InternetGatewayDevice.DeviceInfo.Manufacturer</string></ParameterNames>",
        ),
        (
            "get_object.xml",
            "GetParameterValues",
            "<ParameterNames><string>InternetGatewayDevice.DeviceInfo.</string></ParameterNames>",
        ),
        (
            "get_hidden.xml",
            "GetParameterValues",
            "<ParameterNames><string>InternetGatewayDevice.ManagementServer.Password</string></ParameterNames>",
        ),
        ("fault_unknown.xml", "UnknownMethod", ""),
        (
            "fault_9003.xml",
            "SetParameterValues",
            "<ParameterKey>x</ParameterKey>",
        ),
        (
            "fault_9005.xml",
            "SetParameterValues",
            "<ParameterList><ParameterValueStruct><Name>Does.Not.Exist</Name><Value>x</Value></ParameterValueStruct></ParameterList>",
        ),
        (
            "fault_9008.xml",
            "SetParameterValues",
            "<ParameterList><ParameterValueStruct><Name>InternetGatewayDevice.DeviceInfo.Manufacturer</Name><Value>x</Value></ParameterValueStruct></ParameterList>",
        ),
        (
            "get_ppp.xml",
            "GetParameterValues",
            concat!(
                "<ParameterNames><string>",
                "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANPPPConnection.1.Username",
                "</string></ParameterNames>"
            ),
        ),
    ];
    for (filename, method, fields) in cases {
        snapshot(filename, &request(method, fields));
    }
    let mut cpe = Cpe::new(device().model(&profile()).unwrap());
    snapshot(
        "set_success.xml",
        &cpe.handle(&spv(&[(
            "InternetGatewayDevice.ManagementServer.Username",
            "synthetic-rotated",
        )]))
        .reply
        .unwrap(),
    );
    assert_eq!(
        ppp_prefix(),
        "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANPPPConnection.1."
    );
}

// A broken synthetic fixture must fail this test immediately.
#[allow(clippy::unwrap_used)]
fn normalize_inform(xml: Vec<u8>) -> Vec<u8> {
    let mut text = String::from_utf8(xml).unwrap();
    let from = text.find("<CurrentTime>").unwrap() + "<CurrentTime>".len();
    let to = text[from..].find("</CurrentTime>").unwrap() + from;
    text.replace_range(from..to, "<NORMALIZED>");
    for parameter in [
        "InternetGatewayDevice.DeviceInfo.UpTime",
        "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANIPConnection.3.Uptime",
    ] {
        if let Some(start) = text.find(parameter)
            && let Some(open) = text[start..].find("<Value")
        {
            let open = start + open;
            if let Some(value_start) = text[open..].find('>') {
                let value_start = open + value_start + 1;
                if let Some(value_end) = text[value_start..].find("</Value>") {
                    text.replace_range(value_start..value_start + value_end, "<NORMALIZED>");
                }
            }
        }
    }
    text.into_bytes()
}

#[test]
fn inform_wire_snapshot() {
    let d = device();
    snapshot(
        "inform.xml",
        &normalize_inform(
            soap::inform(&d.model(&profile()).unwrap(), d.inform_parameters()).unwrap(),
        ),
    );
}

#[test]
fn management_address_is_unspecified_in_inform_and_parameter_queries() {
    // The emulator does not know the router's DHCP management address. Keep the
    // parameter and its wire type, using TR-098's empty unspecified IP address.
    let name = "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANIPConnection.3.ExternalIPAddress";
    let d = device();
    assert!(d.template.parameters[name].value.is_empty());
    let expected = format!(
        "<ParameterValueStruct><Name>{name}</Name>\r\n<Value xsi:type=\"xsd:string\"></Value>"
    );
    let inform = soap::inform(&d.model(&profile()).unwrap(), d.inform_parameters()).unwrap();
    assert_eq!(
        String::from_utf8(inform)
            .unwrap()
            .matches(&expected)
            .count(),
        1
    );
    for path in [
        name,
        "InternetGatewayDevice.WANDevice.1.WANConnectionDevice.1.WANIPConnection.3.",
        "",
    ] {
        let fields = format!("<ParameterNames><string>{path}</string></ParameterNames>");
        let reply = request("GetParameterValues", &fields);
        assert_eq!(
            String::from_utf8(reply).unwrap().matches(&expected).count(),
            1
        );
    }
}

#[test]
fn stored_format_snapshots() {
    let root = tempfile::tempdir().unwrap();
    let p = profile();
    let store = Store::open(root.path()).unwrap();
    store.save(&p).unwrap();
    snapshot(
        "profile_v2.json",
        &fs::read(
            root.path()
                .join("devices")
                .join(p.serial.as_ref())
                .join("profile.json"),
        )
        .unwrap(),
    );
    let export = d_export();
    let path = store.save_export(&p.serial, &export).unwrap();
    snapshot("extracted-credentials.json", &fs::read(&path).unwrap());
}

// A broken synthetic fixture must fail this test immediately.
#[allow(clippy::unwrap_used)]
fn d_export() -> Export {
    device()
        .export(&Assignments::from([
            (
                format!("{}Username", ppp_prefix()),
                Secret::new("synthetic-user"),
            ),
            (
                format!("{}Password", ppp_prefix()),
                Secret::new("synthetic-password"),
            ),
        ]))
        .unwrap()
}

#[test]
fn v1_profile_reads_without_rewrite() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path()).unwrap();
    let p = profile();
    store.save(&p).unwrap();
    let path = root
        .path()
        .join("devices")
        .join(p.serial.as_ref())
        .join("profile.json");
    let original = include_bytes!("fixtures/golden/profile_v1.json");
    fs::write(&path, original).unwrap();
    let loaded = store.load(&p.serial).unwrap();
    assert_eq!(loaded.username.expose(), "SYN123456");
    assert_eq!(loaded.password.expose(), "synthetic-wlan");
    assert_eq!(
        loaded.credentials_source,
        yettel_cwmp::domain::profile::CredentialsSource::Server
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    store.save(&loaded).unwrap();
    let migrated: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(migrated["version"], 2);
    assert_eq!(migrated["credentials_source"], "server");

    let mut rotated: serde_json::Value = serde_json::from_slice(original).unwrap();
    rotated["username"] = "rotated-user".into();
    let rotated_bytes = serde_json::to_vec_pretty(&rotated).unwrap();
    fs::write(&path, &rotated_bytes).unwrap();
    let loaded = store.load(&p.serial).unwrap();
    assert_eq!(
        loaded.credentials_source,
        yettel_cwmp::domain::profile::CredentialsSource::Server
    );
    assert_eq!(fs::read(&path).unwrap(), rotated_bytes);
    assert_eq!(
        profile().credentials_source,
        yettel_cwmp::domain::profile::CredentialsSource::Label
    );
}
