//! Atomic CWMP assignment validation and supported method responses.
mod common;

use common::*;
use yettel_cwmp::cwmp::{rpc::Cpe, soap};

#[test]
fn invalid_assignments_are_atomic() {
    let mut cpe = Cpe::new(device().model(&profile()).unwrap());
    cpe.handle(&spv(&[
        (acs_username(), "saved-user"),
        (acs_password(), "saved-password"),
    ]))
    .reply
    .unwrap();
    let received = cpe.received.clone();
    let readonly = "InternetGatewayDevice.DeviceInfo.Manufacturer";
    let unchanged = [acs_username(), acs_password(), parameter_key(), readonly]
        .map(|name| (name, cpe.model.params[name].clone()));
    let missing_value = format!(
        "<ParameterValueStruct><Name>{}</Name></ParameterValueStruct>",
        acs_password()
    );
    let valid_change = format!(
        "<ParameterValueStruct><Name>{}</Name><Value>new-user</Value></ParameterValueStruct>",
        acs_username()
    );
    for (request, fault_code) in [
        "<ParameterKey>new-key</ParameterKey>".to_string(),
        format!("<ParameterList>{missing_value}</ParameterList>"),
        "<ParameterList><ParameterValueStruct><Value>new-password</Value></ParameterValueStruct></ParameterList>".into(),
        format!("<ParameterList>{valid_change}{missing_value}</ParameterList>"),
        format!("<c:ParameterList>{valid_change}</c:ParameterList>"),
        format!("<ParameterList><ParameterValueStruct><Name>{}</Name><c:Value>new-password</c:Value></ParameterValueStruct></ParameterList>", acs_password()),
        format!("<ParameterList><ParameterValueStruct><Name>{}</Name><Value><nested/></Value></ParameterValueStruct></ParameterList>", acs_password()),
        format!("<ParameterList><UnknownStruct><Name>{}</Name><Value>new-password</Value></UnknownStruct></ParameterList>", acs_password()),
    ]
    .map(|fields| (rpc("SetParameterValues", &fields), "9003"))
    .into_iter()
    .chain([
        (spv(&[(acs_password(), "rotated&amp;&lt;"), ("Does.Not.Exist", "x")]), "9005"),
        (spv(&[(acs_password(), "rotated"), (readonly, "bad")]), "9008"),
    ]) {
        let request_text = String::from_utf8_lossy(&request);
        let response = cpe.handle(&request).reply.unwrap();
        let doc = soap::parse(&response).unwrap();
        let method = doc.method().unwrap();
        assert!(method.has_tag_name((soap::SOAP, "Fault")), "{request_text}");
        assert_eq!(
            method
                .descendants()
                .find(|n| n.has_tag_name("FaultCode"))
                .and_then(|n| n.text()),
            Some(fault_code),
            "{request_text}"
        );
        assert_eq!(cpe.received, received, "{request_text}");
        for (name, original) in &unchanged {
            assert_eq!(&cpe.model.params[*name], original, "{request_text}");
        }
    }
}

#[test]
fn empty_assignment_list_and_explicit_empty_value_are_valid() {
    let mut cpe = Cpe::new(device().model(&profile()).unwrap());
    let response = cpe
        .handle(&rpc(
            "SetParameterValues",
            "<ParameterList/><ParameterKey>empty-list-key</ParameterKey>",
        ))
        .reply
        .unwrap();
    let doc = soap::parse(&response).unwrap();
    let method = doc.method().unwrap();
    assert!(method.has_tag_name((soap::CWMP, "SetParameterValuesResponse")));
    assert_eq!(
        method
            .children()
            .find(|n| n.has_tag_name("Status"))
            .and_then(|n| n.text()),
        Some("0")
    );
    assert!(cpe.received.is_empty());
    assert_eq!(cpe.model.params[parameter_key()].value, "empty-list-key");
    cpe.handle(&spv(&[(acs_password(), "")])).reply.unwrap();
    assert_eq!(cpe.received[acs_password()].expose(), "");
    assert_eq!(cpe.model.params[acs_password()].value, "");
}

#[test]
fn non_credentials_remain_readable_without_being_collected() {
    let mut cpe = Cpe::new(device().model(&profile()).unwrap());
    let setting = "InternetGatewayDevice.ManagementServer.PeriodicInformInterval";
    let user = format!("{}Username", ppp_prefix());
    let password = format!("{}Password", ppp_prefix());
    cpe.handle(&spv(&[
        (setting, "1234"),
        (acs_username(), "new-acs-user"),
        (acs_password(), "new-acs-password"),
        (&user, "new-ppp-user"),
        (&password, "new-ppp-password"),
    ]))
    .reply
    .unwrap();

    assert_eq!(cpe.received.len(), 4);
    assert!(!cpe.received.contains_key(setting));
    let response = cpe
        .handle(&rpc(
            "GetParameterValues",
            &format!("<ParameterNames><string>{setting}</string></ParameterNames>"),
        ))
        .reply
        .unwrap();
    let doc = soap::parse(&response).unwrap();
    let method = doc.method().unwrap();
    assert!(method.has_tag_name((soap::CWMP, "GetParameterValuesResponse")));
    assert_eq!(
        method
            .descendants()
            .find(|n| n.has_tag_name("Value"))
            .and_then(|n| n.text()),
        Some("1234")
    );
}

#[test]
fn supported_methods_and_unqualified_arguments() {
    let mut cpe = Cpe::new(device().model(&profile()).unwrap());
    let response = cpe.handle(&rpc("GetRPCMethods", "")).reply.unwrap();
    let doc = soap::parse(&response).unwrap();
    let id = doc.id();
    let method = doc.method().unwrap();
    assert_eq!(id, Some("synthetic-id"));
    assert!(method.has_tag_name((soap::CWMP, "GetRPCMethodsResponse")));
    let methods: Vec<_> = method
        .descendants()
        .filter(|n| n.has_tag_name("string"))
        .filter_map(|n| n.text())
        .collect();
    assert_eq!(
        methods,
        [
            "GetRPCMethods",
            "GetParameterNames",
            "GetParameterValues",
            "SetParameterValues"
        ]
    );

    // RPC methods are namespaced, but their arguments are unqualified.
    let response = cpe
        .handle(&rpc(
            "GetParameterValues",
            "<c:ParameterNames><string>Bad.Path</string></c:ParameterNames>",
        ))
        .reply
        .unwrap();
    let doc = soap::parse(&response).unwrap();
    let method = doc.method().unwrap();
    assert!(method.has_tag_name((soap::CWMP, "GetParameterValuesResponse")));
    let parameters = method
        .children()
        .find(|n| n.has_tag_name("ParameterList"))
        .unwrap();
    assert_eq!(parameters.children().filter(|n| n.is_element()).count(), 0);
    assert!(cpe.received.is_empty());
}
