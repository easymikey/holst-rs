//! Whether output carries color, and the styles that apply it.

use console::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorChoice {
    Plain,
    Colored,
}

impl ColorChoice {
    #[must_use]
    pub(crate) const fn colored_if(on: bool) -> Self {
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
    pub(crate) fn dim(self, text: &str) -> String {
        self.paint(Style::new().dim(), text)
    }

    #[must_use]
    pub(crate) fn cyan_bold(self, text: &str) -> String {
        self.paint(Style::new().cyan().bold(), text)
    }

    #[must_use]
    pub(crate) fn green(self, text: &str) -> String {
        self.paint(Style::new().green(), text)
    }

    #[must_use]
    pub(crate) fn yellow(self, text: &str) -> String {
        self.paint(Style::new().yellow(), text)
    }

    #[must_use]
    pub(crate) fn red(self, text: &str) -> String {
        self.paint(Style::new().red(), text)
    }
}
