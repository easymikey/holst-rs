//! The login page as text: no scripts, no outside files.

use crate::login::page::flow::{Notice, Secret, Shown};

const STYLE: &str = "body{font:16px system-ui,sans-serif;max-width:26rem;margin:4rem auto;\
padding:0 1rem}input,button{font:inherit;padding:.5rem;width:100%;box-sizing:border-box;\
margin:.5rem 0}.note{color:#b00020}";

/// The page for what the visitor is to see next.
pub(super) fn page(shown: &Shown) -> String {
    let body = match shown {
        Shown::Email(notice) => form(
            "Enter your Holst email address. A code is sent to it.",
            notice.as_ref(),
            "<input type=\"email\" name=\"email\" placeholder=\"you@example.com\" \
             required autofocus autocomplete=\"email\">",
            "Continue",
        ),
        Shown::Secret {
            email,
            secret,
            notice,
        } => form(
            &asking(*secret, email),
            notice.as_ref(),
            field(*secret),
            "Log in",
        ),
        Shown::LoggedIn { email } => format!(
            "<p>Logged in as {}. You can close this page.</p>",
            escaped(email)
        ),
    };
    document(&body)
}

/// What a link that is no longer good shows.
pub(super) fn gone() -> String {
    document("<p>This login page has expired. Ask for a new link.</p>")
}

fn document(body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Holst login</title><style>{STYLE}</style></head>\
         <body><h1>Log in to Holst</h1>{body}</body></html>"
    )
}

fn form(ask: &str, notice: Option<&Notice>, input: &str, button: &str) -> String {
    let note = notice
        .map(|notice| format!("<p class=\"note\">{}</p>", escaped(&notice.words())))
        .unwrap_or_default();
    format!(
        "<p>{}</p>{note}<form method=\"post\">{input}<button type=\"submit\">{button}\
         </button></form>",
        escaped(ask)
    )
}

fn asking(secret: Secret, email: &str) -> String {
    match secret {
        Secret::Code => format!("A code was sent to {email}. Enter it here."),
        Secret::Password => format!("Enter the password of {email}."),
    }
}

const CODE_FIELD: &str = "<input name=\"secret\" inputmode=\"numeric\" \
autocomplete=\"one-time-code\" required autofocus>";

const PASSWORD_FIELD: &str = "<input type=\"password\" name=\"secret\" \
autocomplete=\"current-password\" required autofocus>";

const fn field(secret: Secret) -> &'static str {
    match secret {
        Secret::Code => CODE_FIELD,
        Secret::Password => PASSWORD_FIELD,
    }
}

fn escaped(text: &str) -> String {
    text.chars()
        .map(|each| match each {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&#39;".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::escaped;

    #[rstest]
    #[case("a<b>&\"'", "a&lt;b&gt;&amp;&quot;&#39;")]
    #[case("plain", "plain")]
    fn markup_in_text_is_escaped(#[case] text: &str, #[case] shown: &str) {
        assert_eq!(escaped(text), shown);
    }
}
