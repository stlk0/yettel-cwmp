//! CWMP RPC dispatch and atomic model updates.
//! Accepted assignments reach persistence before the next acknowledgment.

use super::{
    model::{Assignments, DataModel, parameter_order},
    soap::{
        CWMP, Element, SOAP, child, envelope, field, node_text, parameter_value_struct, parse,
        soap_array,
    },
};
use crate::{
    domain::secret::Secret,
    error::{Error, Result},
    progress::Rpc,
};
use roxmltree::Node;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

#[derive(Clone, Copy)]
enum FaultCode {
    MethodNotSupported = 9000,
    InvalidArguments = 9003,
    InvalidParameterName = 9005,
    NonWritableParameter = 9008,
}

impl FaultCode {
    fn code(self) -> u16 {
        self as u16
    }

    fn message(self) -> &'static str {
        match self {
            Self::MethodNotSupported => "Method not supported",
            Self::InvalidArguments => "Invalid arguments",
            Self::InvalidParameterName => "Invalid parameter name",
            Self::NonWritableParameter => "Attempt to set a read-only parameter",
        }
    }

    fn fault_actor(self) -> &'static str {
        match self {
            Self::MethodNotSupported => "Server",
            Self::InvalidArguments | Self::InvalidParameterName | Self::NonWritableParameter => {
                "Client"
            }
        }
    }
}

/// A method response, or the CWMP fault that replaces it.
type Reply = std::result::Result<Element, FaultCode>;

fn fault(code: FaultCode) -> Element {
    let mut root = Element::new("SOAP-ENV:Fault");
    root.push(Element::with_text("faultcode", code.fault_actor()));
    root.push(Element::with_text("faultstring", "CWMP fault"));
    let mut detail = Element::new("detail");
    let mut fault = Element::new("cwmp:Fault");
    fault.push(Element::with_text("FaultCode", code.code().to_string()));
    fault.push(Element::with_text("FaultString", code.message()));
    detail.push(fault);
    root.push(detail);
    root
}

/// The observed RPC and its reply or secret-free failure.
/// Reply bytes can contain credentials and deliberately have no Debug representation.
pub struct Handled {
    /// Method classified before dispatch, including methods that produce a fault.
    pub rpc: Rpc,
    /// Encoded acknowledgment/fault, or failure to process an envelope.
    pub reply: Result<Vec<u8>>,
}

/// State for one emulated device session.
pub struct Cpe {
    /// Live parameter model for the session.
    pub model: DataModel,
    /// Accepted credential assignments awaiting persistence/export.
    pub received: Assignments,
    started: Instant,
}

impl Cpe {
    /// Start an emulated device with the validated model.
    pub fn new(model: DataModel) -> Self {
        Self {
            model,
            received: BTreeMap::new(),
            started: Instant::now(),
        }
    }

    /// Parse once, classify the RPC, and produce its wire-compatible response.
    pub fn handle(&mut self, raw: &[u8]) -> Handled {
        let parsed = match parse(raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                return Handled {
                    rpc: Rpc::Unknown,
                    reply: Err(error),
                };
            }
        };
        let method = match parsed.method() {
            Ok(method) => method,
            Err(error) => {
                return Handled {
                    rpc: Rpc::Unknown,
                    reply: Err(error),
                };
            }
        };
        if method.has_tag_name((SOAP, "Fault")) {
            return Handled {
                rpc: Rpc::Unknown,
                reply: Err(Error::Protocol),
            };
        }
        let rpc = classify_rpc(method);
        self.refresh_uptime();
        let response = self.respond(method, rpc).unwrap_or_else(fault);
        Handled {
            rpc,
            reply: envelope(parsed.id(), response),
        }
    }

    fn refresh_uptime(&mut self) {
        let elapsed = self.started.elapsed().as_secs().to_string();
        let updates: Vec<_> = self
            .model
            .params
            .keys()
            .filter_map(|name| {
                let (parent, leaf) = name.rsplit_once('.')?;
                if !leaf.eq_ignore_ascii_case("uptime") {
                    return None;
                }
                let connected = self
                    .model
                    .params
                    .get(&format!("{parent}.ConnectionStatus"))
                    .is_none_or(|p| p.value == "Connected");
                Some((
                    name.clone(),
                    if connected {
                        elapsed.clone()
                    } else {
                        "0".into()
                    },
                ))
            })
            .collect();
        for (name, value) in updates {
            if let Some(parameter) = self.model.params.get_mut(&name) {
                parameter.value = value;
            }
        }
    }

    fn respond(&mut self, request: Node<'_, '_>, rpc: Rpc) -> Reply {
        match rpc {
            Rpc::GetRpcMethods => self.get_rpc_methods(),
            Rpc::GetParameterNames => self.get_parameter_names(request),
            Rpc::GetParameterValues => self.get_parameter_values(request),
            Rpc::SetParameterValues => self.set_parameter_values(request),
            Rpc::None | Rpc::Unknown | Rpc::Unsupported => Err(FaultCode::MethodNotSupported),
        }
    }

    fn get_rpc_methods(&self) -> Reply {
        let mut response = Element::new("cwmp:GetRPCMethodsResponse");
        response.push(soap_array(
            "MethodList",
            "xsd:string",
            [
                "GetRPCMethods",
                "GetParameterNames",
                "GetParameterValues",
                "SetParameterValues",
            ]
            .iter()
            .map(|method| Element::with_text("string", *method))
            .collect(),
        ));
        Ok(response)
    }

    fn get_parameter_names(&self, request: Node<'_, '_>) -> Reply {
        let path = field(request, "ParameterPath");
        let next = matches!(field(request, "NextLevel").trim(), "1" | "true");
        let mut names: Vec<_> = if self.model.params.contains_key(&path) {
            if next {
                return Err(FaultCode::InvalidArguments);
            }
            vec![path.to_string()]
        } else if path.is_empty() || self.model.objects.contains(&path) {
            self.model
                .params
                .keys()
                .chain(self.model.objects.iter())
                .filter(|name| name.starts_with(&path))
                .cloned()
                .collect()
        } else {
            return Err(FaultCode::InvalidParameterName);
        };
        if next {
            names.retain(|name| {
                name != &path && !name[path.len()..].trim_end_matches('.').contains('.')
            });
        }
        names.sort_by(|a, b| parameter_order(a, b));
        let mut response = Element::new("cwmp:GetParameterNamesResponse");
        response.push(soap_array(
            "ParameterList",
            "cwmp:ParameterInfoStruct",
            names
                .iter()
                .map(|name| {
                    let mut item = Element::new("ParameterInfoStruct");
                    item.push(Element::with_text("Name", name));
                    item.push(Element::with_text(
                        "Writable",
                        u8::from(self.model.writable.contains(name)).to_string(),
                    ));
                    item
                })
                .collect(),
        ));
        Ok(response)
    }

    fn get_parameter_values(&self, request: Node<'_, '_>) -> Reply {
        let mut names = BTreeSet::new();
        if let Some(container) = child(request, "ParameterNames") {
            for node in container.children().filter(Node::is_element) {
                let path = node_text(node);
                if self.model.params.contains_key(&path) {
                    names.insert(path.to_string());
                } else if path.is_empty() || self.model.objects.contains(&path) {
                    names.extend(
                        self.model
                            .params
                            .keys()
                            .filter(|name| name.starts_with(&path))
                            .cloned(),
                    );
                } else {
                    return Err(FaultCode::InvalidParameterName);
                }
            }
        }
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort_by(|a, b| parameter_order(a, b));
        let mut response = Element::new("cwmp:GetParameterValuesResponse");
        response.push(soap_array(
            "ParameterList",
            "cwmp:ParameterValueStruct",
            names
                .iter()
                .map(|name| {
                    let parameter = &self.model.params[name];
                    parameter_value_struct(
                        name,
                        &parameter.kind,
                        if self.model.hidden.contains(name) {
                            ""
                        } else {
                            &parameter.value
                        },
                    )
                })
                .collect(),
        ));
        Ok(response)
    }

    fn parameter_changes(
        &self,
        request: Node<'_, '_>,
    ) -> std::result::Result<BTreeMap<String, String>, FaultCode> {
        let mut changes = BTreeMap::new();
        let container = child(request, "ParameterList").ok_or(FaultCode::InvalidArguments)?;
        for node in container.children().filter(Node::is_element) {
            if !node.has_tag_name("ParameterValueStruct") || node.tag_name().namespace().is_some() {
                return Err(FaultCode::InvalidArguments);
            }
            let name = child(node, "Name").ok_or(FaultCode::InvalidArguments)?;
            let value = child(node, "Value").ok_or(FaultCode::InvalidArguments)?;
            if name
                .children()
                .chain(value.children())
                .any(|node| node.is_element())
            {
                return Err(FaultCode::InvalidArguments);
            }
            let name = node_text(name);
            if !self.model.params.contains_key(&name) {
                return Err(FaultCode::InvalidParameterName);
            }
            if !self.model.writable.contains(&name) {
                return Err(FaultCode::NonWritableParameter);
            }
            changes.insert(name, node_text(value));
        }
        Ok(changes)
    }

    fn set_parameter_values(&mut self, request: Node<'_, '_>) -> Reply {
        // Validate the entire batch before changing any model or received value.
        let changes = self.parameter_changes(request)?;
        for (name, value) in &changes {
            if let Some(parameter) = self.model.params.get_mut(name) {
                parameter.value = value.clone();
            }
        }
        if let Some(parameter) = self
            .model
            .params
            .get_mut(&self.model.credentials.parameter_key)
        {
            parameter.value = field(request, "ParameterKey");
        }
        // Other assigned values stay in the model; persistence/export need only these four.
        let paths = &self.model.credentials;
        self.received.extend(
            changes
                .into_iter()
                .filter(|(name, _)| {
                    name == &paths.acs_username
                        || name == &paths.acs_password
                        || name == &paths.ppp_username
                        || name == &paths.ppp_password
                })
                .map(|(name, value)| (name, Secret::new(value))),
        );
        let mut response = Element::new("cwmp:SetParameterValuesResponse");
        response.push(Element::with_text("Status", "0"));
        Ok(response)
    }
}

fn classify_rpc(method: Node<'_, '_>) -> Rpc {
    if method.tag_name().namespace() != Some(CWMP) {
        return Rpc::Unsupported;
    }
    match method.tag_name().name() {
        "GetRPCMethods" => Rpc::GetRpcMethods,
        "GetParameterNames" => Rpc::GetParameterNames,
        "GetParameterValues" => Rpc::GetParameterValues,
        "SetParameterValues" => Rpc::SetParameterValues,
        _ => Rpc::Unsupported,
    }
}
