//! What can go wrong in `holst-mcp --http`.

use std::{
    io,
    net::{AddrParseError, SocketAddr},
};

use crate::http::{MIN_TOKEN_CHARS, TOKEN_FILE, TOKEN_VAR};

#[derive(Debug, thiserror::Error)]
pub(crate) enum HttpError {
    #[error("--http needs an address to listen on, like 127.0.0.1:8808")]
    MissingAddr,

    #[error("{text} is not an address to listen on, like 127.0.0.1:8808")]
    BadAddr {
        text: String,
        #[source]
        source: AddrParseError,
    },

    #[error("unknown argument {arg}: after --http come an address and --print-config")]
    UnknownArg { arg: String },

    #[error(
        "{addr} is not a loopback address and there is no token yet: set \
         {TOKEN_VAR}, or run holst-mcp --http 127.0.0.1:<port> --print-config \
         once to create {TOKEN_FILE}"
    )]
    NoToken { addr: SocketAddr },

    #[error(
        "{TOKEN_VAR} is shorter than {MIN_TOKEN_CHARS} characters: set a longer \
         secret, or unset it to use the token file"
    )]
    ShortToken,

    #[error("there is no place for the token file: set HOLST_HOME")]
    NoTokenPlace,

    #[error("could not read the token file")]
    TokenRead(#[source] holst_core::Error),

    #[error("could not write the token file")]
    TokenWrite(#[source] holst_core::Error),

    #[error("the system has no random bytes for a token")]
    Random(#[source] getrandom::Error),

    #[error("could not listen on {addr}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },
}
