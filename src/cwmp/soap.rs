//! SOAP encoding and parsing for the CWMP exchange.
//! quick-xml writes exact wire bytes; roxmltree reads and validates incoming XML.
use super::model::DataModel;
use crate::error::{Error, Result};
use quick_xml::{
    Writer,
    events::{BytesEnd, BytesStart, BytesText, Event},
};
use roxmltree::{Document, Node, NodeId};

/// SOAP 1.1 envelope namespace.
pub const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
/// CWMP 1.0 method namespace used by the bundled device.
pub const CWMP: &str = "urn:dslforum-org:cwmp-1-0";

// A small construction helper over quick-xml; escaping and XML encoding belong to the library.
/// Small owned XML element used to preserve the device’s wire format.
pub struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    text: Option<String>,
    pub(super) children: Vec<Element>,
}
impl Element {
    /// Construct an empty named element.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            attrs: vec![],
            text: None,
            children: vec![],
        }
    }
    /// Construct a named element whose text is escaped during encoding.
    pub fn with_text(name: &str, value: impl Into<String>) -> Self {
        let mut e = Self::new(name);
        e.text = Some(value.into());
        e
    }
    /// Append a child in wire order.
    pub fn push(&mut self, element: Self) {
        self.children.push(element);
    }
    pub(super) fn attr(mut self, name: &str, value: impl Into<String>) -> Self {
        self.attrs.push((name.into(), value.into()));
        self
    }
    // Writer<Vec<u8>> has no external I/O: a write failure is an internal serialization invariant.
    fn write(&self, w: &mut Writer<Vec<u8>>) -> Result<()> {
        let mut start = BytesStart::new(&self.name);
        for (name, value) in &self.attrs {
            start.push_attribute((name.as_str(), value.as_str()));
        }
        w.write_event(Event::Start(start))
            .map_err(|_| Error::Internal)?;
        if let Some(text) = &self.text {
            w.write_event(Event::Text(BytesText::new(text)))
                .map_err(|_| Error::Internal)?;
        }
        for child in &self.children {
            child.write(w)?;
        }
        w.write_event(Event::End(BytesEnd::new(&self.name)))
            .map_err(|_| Error::Internal)?;
        w.get_mut().extend_from_slice(b"\r\n");
        Ok(())
    }
}
pub(super) fn soap_array(name: &str, kind: &str, items: Vec<Element>) -> Element {
    let mut element =
        Element::new(name).attr("SOAP-ENC:arrayType", format!("{kind}[{}]", items.len()));
    element.children = items;
    element
}
pub(super) fn parameter_value_struct(name: &str, kind: &str, text: &str) -> Element {
    let mut item = Element::new("ParameterValueStruct");
    item.push(Element::with_text("Name", name));
    item.push(Element::with_text("Value", text).attr("xsi:type", kind));
    item
}
/// Encode one SOAP method with an optional CWMP correlation identifier.
pub fn envelope(id: Option<&str>, method: Element) -> Result<Vec<u8>> {
    let mut root = Element::new("SOAP-ENV:Envelope");
    for (prefix, ns) in [
        ("SOAP-ENV", SOAP),
        ("SOAP-ENC", "http://schemas.xmlsoap.org/soap/encoding/"),
        ("xsi", "http://www.w3.org/2001/XMLSchema-instance"),
        ("xsd", "http://www.w3.org/2001/XMLSchema"),
        ("cwmp", CWMP),
    ] {
        root = root.attr(&format!("xmlns:{prefix}"), ns);
    }
    if let Some(id) = id {
        let mut header = Element::new("SOAP-ENV:Header");
        header.push(Element::with_text("cwmp:ID", id).attr("SOAP-ENV:mustUnderstand", "1"));
        root.push(header);
    }
    let mut body = Element::new("SOAP-ENV:Body");
    body.push(method);
    root.push(body);
    let mut writer = Writer::new(Vec::new());
    root.write(&mut writer)?;
    Ok(writer.into_inner())
}
/// A validated SOAP envelope owning its XML document and stable node identifier.
/// NodeId avoids a self-referential document/node structure.
pub struct Envelope<'input> {
    doc: Document<'input>,
    id: Option<String>,
    method: NodeId,
}

impl<'input> Envelope<'input> {
    /// Optional correlation identifier from the header.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// The single validated body method, borrowed from this envelope.
    pub fn method(&self) -> Result<Node<'_, 'input>> {
        self.doc.get_node(self.method).ok_or(Error::Internal)
    }
}

/// Parse and validate the envelope once, retaining its header and method.
pub fn parse(raw: &[u8]) -> Result<Envelope<'_>> {
    let text = std::str::from_utf8(raw).map_err(|_| Error::Protocol)?;
    let doc = Document::parse(text).map_err(|_| Error::Protocol)?;
    let (id, method) = split_envelope(&doc)?;
    let method = method.id();
    Ok(Envelope { doc, id, method })
}
fn split_envelope<'a, 'input>(
    doc: &'a Document<'input>,
) -> Result<(Option<String>, Node<'a, 'input>)> {
    let root = doc.root_element();
    if !root.has_tag_name((SOAP, "Envelope")) {
        return Err(Error::Protocol);
    }
    let body = root
        .children()
        .find(|n| n.has_tag_name((SOAP, "Body")))
        .ok_or(Error::Protocol)?;
    let mut children = body.children().filter(Node::is_element);
    let method = children.next().ok_or(Error::Protocol)?;
    if children.next().is_some() {
        return Err(Error::Protocol);
    }
    let id = root
        .children()
        .find(|n| n.has_tag_name((SOAP, "Header")))
        .and_then(|n| n.children().find(|n| n.has_tag_name((CWMP, "ID"))))
        .map(node_text);
    Ok((id, method))
}
pub(super) fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.has_tag_name(name) && n.tag_name().namespace().is_none())
}
pub(super) fn node_text(node: Node<'_, '_>) -> String {
    node.children()
        .take_while(|n| !n.is_element())
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}
pub(super) fn field(node: Node<'_, '_>, name: &str) -> String {
    child(node, name).map(node_text).unwrap_or_default()
}
/// Build the opening Inform with the configured ordered parameter list.
pub fn inform(model: &DataModel, names: &[String]) -> Result<Vec<u8>> {
    let mut rpc = Element::new("cwmp:Inform");
    let mut id = Element::new("DeviceId");
    for (tag, param) in [
        ("Manufacturer", "Manufacturer"),
        ("OUI", "ManufacturerOUI"),
        ("ProductClass", "ProductClass"),
        ("SerialNumber", "SerialNumber"),
    ] {
        id.push(Element::with_text(
            tag,
            &model
                .params
                .get(&format!("InternetGatewayDevice.DeviceInfo.{param}"))
                .ok_or(Error::Internal)?
                .value,
        ));
    }
    rpc.push(id);
    rpc.push(soap_array(
        "Event",
        "cwmp:EventStruct",
        ["0 BOOTSTRAP", "1 BOOT"]
            .iter()
            .map(|code| {
                let mut e = Element::new("EventStruct");
                e.push(Element::with_text("EventCode", *code));
                e.push(Element::with_text("CommandKey", ""));
                e
            })
            .collect(),
    ));
    rpc.push(Element::with_text("MaxEnvelopes", "1"));
    let now = time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .map_err(|_| Error::Internal)?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| Error::Internal)?;
    rpc.push(Element::with_text("CurrentTime", now));
    rpc.push(Element::with_text("RetryCount", "0"));
    let mut values = vec![];
    for name in names {
        let parameter = model.params.get(name).ok_or(Error::Internal)?;
        values.push(parameter_value_struct(
            name,
            &parameter.kind,
            &parameter.value,
        ));
    }
    rpc.push(soap_array(
        "ParameterList",
        "cwmp:ParameterValueStruct",
        values,
    ));
    envelope(Some("1"), rpc)
}
