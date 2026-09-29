//! Secret-free capture progress shared by the protocol session and interfaces.
/// Current high-level session operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    /// Acquire and validate the saved profile.
    Profile,
    /// Send the opening device announcement.
    Inform,
    /// Exchange provider requests and device replies.
    Session,
    /// Validate and save received internet settings.
    Export,
}
/// Last observed provider method, without request data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rpc {
    #[default]
    /// No provider method has been observed yet.
    None,
    /// The message could not be classified safely.
    Unknown,
    /// Provider requests supported methods.
    GetRpcMethods,
    /// Provider requests parameter names and writability.
    GetParameterNames,
    /// Provider requests visible parameter values.
    GetParameterValues,
    /// Provider assigns parameter values.
    SetParameterValues,
    /// A well-formed method is unsupported.
    Unsupported,
}
/// Secret-free facts passed from the session to its interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Current operation.
    pub stage: Stage,
    /// Most recently classified provider method.
    pub rpc: Rpc,
}
