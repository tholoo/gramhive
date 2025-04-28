use grammers_client::InputMessage;
use htmlescape::encode_minimal;
use std::fmt::Write;
use std::ops::Deref;

#[derive(Debug, Default, Clone)]
pub struct TgHtml {
    content: String,
}

pub fn tg_html() -> TgHtml {
    TgHtml::new()
}

impl TgHtml {
    pub fn new() -> Self {
        Self {
            content: String::new(),
        }
    }
    /// Append normal (unformatted) text.
    pub fn append<T: AsRef<str>>(mut self, other: T) -> Self {
        self.content.push_str(other.as_ref());
        self
    }

    /// Append normal (unformatted) text.
    pub fn plain<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(self.content, "{}", encode_minimal(text.as_ref())).unwrap();
        self
    }

    /// Append bold text.
    pub fn bold<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(self.content, "<b>{}</b>", encode_minimal(text.as_ref())).unwrap();
        self
    }

    /// Append blockquote text.
    pub fn blockquote<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(
            self.content,
            "<blockquote expandable>{}</blockquote>",
            text.as_ref()
        )
        .unwrap();
        self
    }

    /// Append italic text.
    pub fn italic<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(self.content, "<i>{}</i>", encode_minimal(text.as_ref())).unwrap();
        self
    }

    /// Append underlined text.
    pub fn underline<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(self.content, "<u>{}</u>", encode_minimal(text.as_ref())).unwrap();
        self
    }

    /// Append strikethrough text.
    pub fn strikethrough<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(self.content, "<s>{}</s>", encode_minimal(text.as_ref())).unwrap();
        self
    }

    /// Append spoiler text.
    pub fn spoiler<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(
            self.content,
            "<details>{}</details>",
            encode_minimal(text.as_ref())
        )
        .unwrap();
        self
    }

    /// Append inline code.
    pub fn code<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(
            self.content,
            "<code>{}</code>",
            encode_minimal(text.as_ref())
        )
        .unwrap();
        self
    }

    /// Append a preformatted text block.
    pub fn pre<T: AsRef<str>>(mut self, text: T) -> Self {
        write!(self.content, "<pre>{}</pre>", encode_minimal(text.as_ref())).unwrap();
        self
    }

    /// Append a hyperlink.
    pub fn link<T: AsRef<str>, U: AsRef<str>>(mut self, text: U, url: T) -> Self {
        write!(
            self.content,
            "<a href=\"{}\">{}</a>",
            url.as_ref(),
            encode_minimal(text.as_ref())
        )
        .unwrap();
        self
    }

    // Mention a user
    pub fn mention<U: AsRef<str>, T: Into<i64>>(self, text: U, user_id: T) -> Self {
        self.link(text, format!("tg://user?id={}", user_id.into()))
    }

    /// Append newline(s) to the content.
    ///
    /// The parameter `count` is the number of newline characters to append.
    /// If `count` is 0 then nothing is added.
    pub fn n(mut self, count: usize) -> Self {
        for _ in 0..count {
            self.content.push('\n');
        }
        self
    }

    /// Append space(s) to the content.
    ///
    /// The parameter `count` is the number of space characters to append.
    pub fn s(mut self, count: usize) -> Self {
        for _ in 0..count {
            self.content.push(' ');
        }
        self
    }

    /// Consume the builder and return the final HTML string.
    pub fn build(self) -> String {
        self.content
    }

    /// Convert this HTML builder into an InputMessage by calling InputMessage::html.
    ///
    /// This uses your internal parser (via InputMessage::html) to extract text and entities.
    pub fn into_message(self) -> InputMessage {
        InputMessage::html(self.as_ref())
    }
}

/// Allow &Html to be used as a &str.
impl Deref for TgHtml {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

/// Implement AsRef<str> so that Html can be passed where an AsRef<str> is required.
impl AsRef<str> for TgHtml {
    fn as_ref(&self) -> &str {
        &self.content
    }
}

/// Allow conversion from Html into InputMessage so that functions expecting Into<InputMessage> work seamlessly.
impl From<TgHtml> for InputMessage {
    fn from(html: TgHtml) -> Self {
        InputMessage::html(html.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A helper function that takes a &str.
    fn takes_str(s: &str) -> &str {
        s
    }

    #[test]
    fn test_html_builder() {
        let result = TgHtml::new()
            .plain("Hello, ")
            .bold("World")
            .n(1)
            .italic("!")
            .n(2)
            .underline("Underlined")
            .plain(" ")
            .strikethrough("Striked")
            .n(0) // n(0) should not change anything.
            .link("Link", "https://example.com")
            .build();

        let expected = "Hello, <b>World</b>\n<i>!</i>\n\n<u>Underlined</u> <s>Striked</s><a href=\"https://example.com\">Link</a>";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_as_str() {
        let builder = TgHtml::new().plain("Test ").bold("string");
        // Using Deref so that &builder is a &str:
        let s: &str = &builder;
        // Using AsRef:
        let s2: &str = builder.as_ref();
        assert_eq!(s, "Test <b>string</b>");
        assert_eq!(s2, "Test <b>string</b>");
        // Passing the builder to a function expecting &str.
        assert_eq!(takes_str(&builder), "Test <b>string</b>");
    }

    #[test]
    fn test_generic_input() {
        // Test using a String instead of a &str.
        let my_string = String::from("Hello, world!");
        let result = TgHtml::new().bold(my_string).build();
        let expected = format!("<b>{}</b>", "Hello, world!");
        assert_eq!(result, expected);
    }
}
