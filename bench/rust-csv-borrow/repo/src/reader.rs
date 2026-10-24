use crate::error::Error;

/// Records of an input, one per line, fields split on commas.
pub struct Reader<'a> {
    lines: std::str::Lines<'a>,
}

impl<'a> Reader<'a> {
    pub fn new(input: &'a str) -> Self {
        Reader { lines: input.lines() }
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Vec<String>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.lines.next()?;
        Some(Ok(line.split(',').map(String::from).collect()))
    }
}
