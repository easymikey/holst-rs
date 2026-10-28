//! What the comment commands print.

use holst_core::{
    command::comment::{CommentReport, ThreadList},
    domain::text::{EMPTY, joined},
};

use crate::{
    error::Result,
    output::{self, OutputFormat},
};

pub(super) fn listing(format: OutputFormat, list: &ThreadList) -> Result<()> {
    let text = if list.threads.is_empty() {
        EMPTY.to_owned()
    } else {
        joined(list.lines(), "\n").to_string()
    };
    output::print(format, &text, list)
}

pub(super) fn change(format: OutputFormat, changed: &CommentReport) -> Result<()> {
    output::print(format, &changed.phrase(), changed)
}
