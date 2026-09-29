#![allow(dead_code)]
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};
use yettel_cwmp::{
    cwmp::{
        model::Device,
        soap::{self, Element},
    },
    domain::profile::Profile,
    error::Result,
    net::{PostKind, Transport},
};
// A broken synthetic fixture must fail this test immediately.
#[allow(clippy::unwrap_used)]
pub fn profile() -> Profile {
    Profile::new(
        "SYN123456".parse().unwrap(),
        "02:11:22:33:44:55".parse().unwrap(),
        "synthetic-wlan".into(),
    )
    .unwrap()
}
// A broken synthetic fixture must fail this test immediately.
#[allow(clippy::unwrap_used)]
pub fn device() -> Device {
    let mut d = Device::bundled().unwrap();
    d.provider.acs.host = "localhost".into();
    d.provider.acs.port = 12345;
    d.provider.acs.path = "/synthetic".into();
    d
}
// A broken bundled catalog must fail this test immediately.
#[allow(clippy::unwrap_used)]
fn bundled() -> &'static Device {
    static DEVICE: OnceLock<Device> = OnceLock::new();
    DEVICE.get_or_init(|| Device::bundled().unwrap())
}
/// The bundled router's management username parameter path.
pub fn acs_username() -> &'static str {
    &bundled().template.credentials.acs_username
}
/// The bundled router's management password parameter path.
pub fn acs_password() -> &'static str {
    &bundled().template.credentials.acs_password
}
/// The bundled router's parameter-key path.
pub fn parameter_key() -> &'static str {
    &bundled().template.credentials.parameter_key
}
/// The shared path prefix of the bundled router's internet credentials.
// A broken bundled catalog must fail this test immediately.
#[allow(clippy::unwrap_used)]
pub fn ppp_prefix() -> &'static str {
    bundled()
        .template
        .credentials
        .ppp_username
        .strip_suffix("Username")
        .unwrap()
}
pub fn rpc(name: &str, fields: &str) -> Vec<u8> {
    format!("<s:Envelope xmlns:s='{}' xmlns:c='{}'><s:Header><c:ID>synthetic-id</c:ID></s:Header><s:Body><c:{name}>{fields}</c:{name}></s:Body></s:Envelope>",soap::SOAP,soap::CWMP).into_bytes()
}
pub fn spv(values: &[(&str, &str)]) -> Vec<u8> {
    let fields = values
        .iter()
        .map(|(n, v)| {
            format!(
                "<ParameterValueStruct><Name>{n}</Name><Value>{v}</Value></ParameterValueStruct>"
            )
        })
        .collect::<String>();
    rpc(
        "SetParameterValues",
        &format!(
            "<ParameterList>{fields}</ParameterList><ParameterKey>synthetic-key</ParameterKey>"
        ),
    )
}
// A broken synthetic fixture must fail this test immediately.
#[allow(clippy::unwrap_used)]
pub fn inform_response() -> Vec<u8> {
    soap::envelope(Some("1"), Element::new("cwmp:InformResponse")).unwrap()
}
pub struct Script {
    pub replies: VecDeque<Result<Vec<u8>>>,
    pub posts: Arc<Mutex<Vec<Vec<u8>>>>,
}
impl Script {
    pub fn new(replies: Vec<Result<Vec<u8>>>) -> Self {
        Self {
            replies: replies.into(),
            posts: Arc::default(),
        }
    }
}
impl Transport for Script {
    // Poisoned capture state or an extra request invalidates the scripted test.
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn post(&mut self, body: &[u8], _: PostKind) -> Result<Vec<u8>> {
        self.posts.lock().unwrap().push(body.into());
        self.replies
            .pop_front()
            .expect("unexpected scripted request")
    }
}
