//! Whether output carries color, and the styles that apply it.

use console::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    Plain,
    Colored,
}

impl ColorChoice {
    #[must_use]
    pub const fn colored_if(on: bool) -> Self {
        if on {
            ColorChoice::Colored
        } else {
            ColorChoice::Plain
        }
    }

    fn paint(self, style: Style, text: &str) -> String {
        style
            .force_styling(self == ColorChoice::Colored)
            .apply_to(text)
            .to_string()
    }

    #[must_use]
    pub fn dim(self, text: &str) -> String {
        self.paint(Style::new().dim(), text)
    }

    #[must_use]
    pub fn cyan(self, text: &str) -> String {
        self.paint(Style::new().cyan(), text)
    }

    #[must_use]
    pub fn cyan_bold(self, text: &str) -> String {
        self.paint(Style::new().cyan().bold(), text)
    }

    #[must_use]
    pub fn green(self, text: &str) -> String {
        self.paint(Style::new().green(), text)
    }

    #[must_use]
    pub fn yellow(self, text: &str) -> String {
        self.paint(Style::new().yellow(), text)
    }

    #[must_use]
    pub fn struck(self, text: &str) -> String {
        self.paint(Style::new().dim().strikethrough(), text)
    }

    #[must_use]
    pub fn red(self, text: &str) -> String {
        self.paint(Style::new().red(), text)
    }

    #[must_use]
    pub fn magenta(self, text: &str) -> String {
        self.paint(Style::new().magenta(), text)
    }

    #[must_use]
    pub fn reverse(self, text: &str) -> String {
        self.paint(Style::new().reverse(), text)
    }

    #[must_use]
    pub fn code(self, text: &str) -> String {
        self.code_on(text, str::to_owned)
    }

    #[must_use]
    pub fn code_on(self, text: &str, rest: impl Fn(&str) -> String) -> String {
        match self {
            ColorChoice::Colored => text
                .split('`')
                .enumerate()
                .map(|(at, part)| match at % 2 {
                    1 => self.cyan(part),
                    _ => rest(part),
                })
                .collect(),
            ColorChoice::Plain => rest(text),
        }
    }
}
