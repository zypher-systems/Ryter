use std::borrow::Cow;

use crate::error::Error;

/// Records of an input: fields split on the delimiter, quoted fields read
/// whole, borrowed from the input wherever the bytes are already there.
pub struct Reader<'a> {
    input: &'a str,
    pos: usize,
    line: usize,
    column: usize,
    delimiter: char,
    done: bool,
}

impl<'a> Reader<'a> {
    pub fn new(input: &'a str) -> Self {
        let input = input.strip_prefix('\u{feff}').unwrap_or(input);
        Reader { input, pos: 0, line: 1, column: 1, delimiter: ',', done: false }
    }

    pub fn with_delimiter(mut self, delimiter: char) -> Self {
        self.delimiter = delimiter;
        self
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn at_line_end(&self) -> bool {
        match self.peek() {
            None | Some('\n') => true,
            Some('\r') => self.input[self.pos..].starts_with("\r\n"),
            _ => false,
        }
    }

    fn consume_line_end(&mut self) {
        if self.input[self.pos..].starts_with("\r\n") {
            self.bump();
        }
        if self.peek() == Some('\n') {
            self.bump();
        }
    }

    fn record(&mut self) -> Result<Vec<Cow<'a, str>>, Error> {
        let start_line = self.line;
        let mut fields = Vec::new();
        loop {
            let field = if self.peek() == Some('"') {
                self.quoted(start_line)?
            } else {
                self.unquoted()
            };
            fields.push(field);
            if self.peek() == Some(self.delimiter) {
                self.bump();
                continue;
            }
            self.consume_line_end();
            return Ok(fields);
        }
    }

    fn unquoted(&mut self) -> Cow<'a, str> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == self.delimiter || self.at_line_end() {
                break;
            }
            self.bump();
        }
        Cow::Borrowed(&self.input[start..self.pos])
    }

    fn quoted(&mut self, start_line: usize) -> Result<Cow<'a, str>, Error> {
        self.bump();
        let start = self.pos;
        let mut owned: Option<String> = None;
        loop {
            match self.bump() {
                None => return Err(Error::UnterminatedQuote { line: start_line }),
                Some('"') if self.peek() == Some('"') => {
                    let before = self.pos - 1;
                    owned.get_or_insert_with(|| self.input[start..before].to_string()).push('"');
                    self.bump();
                }
                Some('"') => {
                    let end = self.pos - 1;
                    if !(self.peek() == Some(self.delimiter) || self.at_line_end()) {
                        return Err(Error::UnexpectedAfterQuote { line: self.line, column: self.column });
                    }
                    return Ok(match owned {
                        Some(text) => Cow::Owned(text),
                        None => Cow::Borrowed(&self.input[start..end]),
                    });
                }
                Some(c) => {
                    if let Some(text) = owned.as_mut() {
                        text.push(c);
                    }
                }
            }
        }
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Vec<Cow<'a, str>>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.pos >= self.input.len() {
            self.done = true;
            return None;
        }
        match self.record() {
            Ok(fields) => Some(Ok(fields)),
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}
