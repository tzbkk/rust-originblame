use super::{ParseResult, Parser};
use anyhow::Result;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

pub struct MediawikiParser;

fn safe_filename(title: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    base64::engine::Engine::encode(&URL_SAFE_NO_PAD, title.as_bytes())
        .chars()
        .take(200)
        .collect::<String>()
}

impl Parser for MediawikiParser {
    fn parse(&self, ob_dir: &Path, file: &Path, license: &str, split: bool) -> Result<ParseResult> {
        let content = std::fs::read_to_string(file)?;
        let mut reader = Reader::from_str(&content);

        let mut result = ParseResult::default();
        let mut seen_authors: HashSet<String> = HashSet::new();

        let mut in_page = false;
        let mut in_title = false;
        let mut in_ns = false;
        let mut in_username = false;
        let mut in_ip = false;
        let mut in_timestamp = false;
        let mut in_text = false;

        let mut page_title = String::new();
        let mut page_ns = String::new();
        let mut page_year = String::new();
        let mut page_text = String::new();
        let mut page_contributors: HashSet<String> = HashSet::new();

        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Start(e) => {
                    let name = e.name();
                    let tag = String::from_utf8_lossy(name.as_ref());
                    match tag.as_ref() {
                        "page" => {
                            in_page = true;
                            page_title.clear();
                            page_ns.clear();
                            page_year.clear();
                            page_text.clear();
                            page_contributors.clear();
                        }
                        "title" if in_page => in_title = true,
                        "ns" if in_page => in_ns = true,
                        "username" => in_username = true,
                        "ip" => in_ip = true,
                        "timestamp" if in_page => in_timestamp = true,
                        "text" if in_page => in_text = true,
                        _ => {}
                    }
                }
                Event::Text(e) => {
                    let text = e.unescape()?.to_string();
                    if in_title {
                        page_title = text.clone();
                    } else if in_ns {
                        page_ns = text.clone();
                    } else if in_username {
                        page_contributors.insert(text.clone());
                    } else if in_ip {
                        page_contributors.insert(text.clone());
                    } else if in_timestamp {
                        page_year = text.chars().take(4).collect();
                    } else if in_text {
                        page_text = text;
                    }
                }
                Event::End(e) => {
                    let ename = e.name();
                    let tag = String::from_utf8_lossy(ename.as_ref());
                    match tag.as_ref() {
                        "title" => in_title = false,
                        "ns" => in_ns = false,
                        "username" => in_username = false,
                        "ip" => in_ip = false,
                        "timestamp" => in_timestamp = false,
                        "text" => in_text = false,
                        "page" => {
                            in_page = false;

                            if page_ns != "0" {
                                buf.clear();
                                continue;
                            }

                            result.pages_parsed += 1;

                            let mut author_ids: Vec<String> = Vec::new();
                            for name in &page_contributors {
                                let email = format!("{}@mediawiki", name);
                                let aid = crate::authors::author_id(name, &email);
                                if seen_authors.insert(aid.clone()) {
                                    let _ = crate::authors::add(ob_dir, name, &email);
                                    result.authors_registered += 1;
                                }
                                author_ids.push(aid);
                            }

                            if !author_ids.is_empty() {
                                let _section = crate::register::register_section(
                                    ob_dir,
                                    &format!("raw/{}", page_title),
                                    &author_ids,
                                    &[],
                                    license,
                                    &page_year,
                                );
                                result.sections_created += 1;
                            }

                            if split {
                                let split_dir = ob_dir.join(".ob").join("split");
                                std::fs::create_dir_all(&split_dir)?;
                                let filename = safe_filename(&page_title);
                                let split_path = split_dir.join(format!("{}.xml", filename));
                                let mut f = std::fs::File::create(&split_path)?;
                                write!(f, "<page>")?;
                                write!(f, "<title>{}</title>", page_title)?;
                                write!(f, "<ns>0</ns>")?;
                                write!(f, "</page>")?;
                                result.split_files_created += 1;
                            }
                        }
                        _ => {}
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }

        Ok(result)
    }
}
