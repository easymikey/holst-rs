//! Whether output carries color, and the styles that apply it.

use console::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coloring {
    Plain,
    Colored,
}

impl Coloring {
    pub(crate) const fn colored_if(on: bool) -> Self {
        if on {
            Coloring::Colored
        } else {
            Coloring::Plain
        }
    }

    fn paint(self, style: Style, text: &str) -> String {
        style
            .force_styling(self == Coloring::Colored)
            .apply_to(text)
            .to_string()
    }

    pub(crate) fn dim(self, text: &str) -> String {
        self.paint(Style::new().dim(), text)
    }

    pub(crate) fn cyan(self, text: &str) -> String {
        self.paint(Style::new().cyan(), text)
    }

    pub(crate) fn green(self, text: &str) -> String {
        self.paint(Style::new().green(), text)
    }

    pub(crate) fn yellow(self, text: &str) -> String {
        self.paint(Style::new().yellow(), text)
    }

    pub(crate) fn struck(self, text: &str) -> String {
        self.paint(Style::new().dim().strikethrough(), text)
    }

    pub(crate) fn red(self, text: &str) -> String {
        self.paint(Style::new().red(), text)
    }

    pub(crate) fn magenta(self, text: &str) -> String {
        self.paint(Style::new().magenta(), text)
    }

    pub(crate) fn reverse(self, text: &str) -> String {
        self.paint(Style::new().reverse(), text)
    }

    pub(crate) fn code(self, text: &str) -> String {
        self.code_on(text, str::to_owned)
    }

    pub(crate) fn code_on(self, text: &str, rest: impl Fn(&str) -> String) -> String {
        match self {
            Coloring::Colored => text
                .split('`')
                .enumerate()
                .map(|(at, part)| match at % 2 {
                    1 => self.cyan(part),
                    _ => rest(part),
                })
                .collect(),
            Coloring::Plain => rest(text),
        }
    }
}
