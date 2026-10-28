use serde_json::Value;

use crate::{
    effect::{http::BoxFuture, request::ApiRequest, server::HolstServer},
    error::Result,
};

/// What came of a planned request.
#[derive(Debug, Clone, PartialEq)]
pub enum CarriedRequest {
    Sent(Value),
    Reported(ApiRequest),
}

/// Sends, reports or records a planned request; the caller's edge picks which.
pub trait RequestInterpreter: Sync {
    /// Carries out `request` on `server`.
    ///
    /// # Errors
    /// As [`HolstServer::send`] for an interpreter that sends.
    fn carry<'a>(
        &'a self,
        server: &'a dyn HolstServer,
        request: ApiRequest,
    ) -> BoxFuture<'a, Result<CarriedRequest>>;
}
