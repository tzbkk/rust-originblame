pub mod mediawiki;

use anyhow::Result;
use std::path::Path;

#[derive(Debug, Default)]
pub struct ParseResult {
    pub pages_parsed: usize,
    pub authors_registered: usize,
    pub sections_created: usize,
    pub split_files_created: usize,
}

pub trait Parser {
    fn parse(&self, ob_dir: &Path, file: &Path, license: &str, split: bool) -> Result<ParseResult>;
}

pub fn get_parser(name: &str) -> Option<Box<dyn Parser>> {
    match name {
        "mediawiki" => Some(Box::new(mediawiki::MediawikiParser)),
        _ => None,
    }
}
