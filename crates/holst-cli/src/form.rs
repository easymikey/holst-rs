//! The form lives in `holst-form`; these are the names the commands use.

#[cfg(test)] pub(crate) use holst_form::fake;
pub(crate) use holst_form::{
    draw,
    form::{Form, Question},
    line,
    prompt,
    terminal,
};
