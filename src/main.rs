use clap::{Parser, Subcommand};

use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "ob",
    version,
    about = "Record- and token-level provenance tracking for AI training datasets"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    #[command(name = "author.add")]
    AuthorAdd {
        name: String,
        email: String,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    #[command(name = "register.add")]
    RegisterAdd {
        #[arg(long)]
        path: String,
        #[arg(long, value_delimiter = ',')]
        authors: Vec<String>,
        #[arg(long, default_value = "")]
        license: String,
        #[arg(long, default_value = "")]
        year: String,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Blame {
        file: PathBuf,
        line_num: usize,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Show {
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        section: Option<String>,
        #[arg(long)]
        license: Option<String>,
        #[arg(long)]
        revoked: bool,
        #[arg(long)]
        tokenizer: Option<String>,
        #[arg(long)]
        index: bool,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Revoke {
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        section: Option<String>,
        #[arg(long)]
        tokenizer: Option<String>,
        #[arg(long)]
        reverse: bool,
        #[arg(long)]
        line_hash: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Purge {
        file: PathBuf,
        #[arg(long)]
        index: bool,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        reverse: bool,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Status {
        #[arg(long)]
        tokenizer: Option<String>,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Clean {
        #[arg(long)]
        split: bool,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Merge {
        #[arg(long)]
        absorb: PathBuf,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Index {
        #[command(subcommand)]
        command: IndexCommands,
    },
    #[cfg(feature = "util")]
    #[command(name = "export-copyright")]
    ExportCopyright {
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        data_file: Option<Vec<String>>,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    #[cfg(feature = "util")]
    Reconcile {
        data_file: PathBuf,
        #[arg(short, long)]
        model: Option<String>,
        #[arg(short, long, default_value = "0.85")]
        threshold: f64,
        #[arg(short, long)]
        embedding_api: Option<String>,
        #[arg(long)]
        compute_all_embeddings: bool,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    #[cfg(feature = "util")]
    Parse {
        #[arg(long)]
        parser: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = "CC-BY-SA-4.0")]
        license: String,
        #[arg(long)]
        split: bool,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    #[command(name = "generate-set")]
    GenerateSet {
        #[arg(long)]
        tokenizer: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
    Version,
}

#[derive(Subcommand)]
enum IndexCommands {
    Build {
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Helper: resolve --email or --author to an author name
    fn resolve_author(dir: &Path, author: Option<&str>, email: Option<&str>) -> Result<String> {
        if let Some(e) = email {
            let found = originblame::authors::query(dir, None, Some(e))?;
            if found.is_empty() {
                anyhow::bail!("No author found with email '{}'", e);
            }
            Ok(found[0].name.clone())
        } else if let Some(a) = author {
            Ok(a.to_string())
        } else {
            anyhow::bail!("--author or --email is required");
        }
    }

    match cli.command {
        Commands::Init { path } => {
            let ob = originblame::ObDir::new(&path);
            ob.init()?;
            println!("Initialized .ob/ in {}", path.display());
        }
        Commands::AuthorAdd { name, email, dir } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let author = originblame::authors::add(&dir, &name, &email)?;
            originblame::oplog::append(&dir, "author_add", &format!("{} <{}>", name, email))?;
            println!("Added author: {} ({})", author.name, &author.id[..8]);
        }
        Commands::RegisterAdd {
            path,
            authors,
            license,
            year,
            dir,
        } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let mut author_ids: Vec<String> = Vec::new();
            for a in &authors {
                let found = originblame::authors::query(&dir, None, Some(a))?;
                if let Some(author) = found.into_iter().next() {
                    author_ids.push(author.id);
                } else {
                    let found = originblame::authors::query(&dir, Some(a), None)?;
                    if let Some(author) = found.into_iter().next() {
                        author_ids.push(author.id);
                    } else {
                        anyhow::bail!("Author '{}' not found. Run `ob author.add` first.", a);
                    }
                }
            }
            let section = originblame::register::register_section(&dir, &path, &author_ids, &author_ids, &license, &year)?;
            originblame::oplog::append(
                &dir,
                "register_section",
                &format!("{} -> {}", path, &section.section_hash[..8]),
            )?;
            println!("Added section: {}", &section.section_hash[..8]);
        }
        Commands::Blame {
            file,
            line_num,
            dir,
        } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");

            let line_content =
                if let Some(bin_idx) = originblame::binary_index::BinaryIndex::open(&dir)? {
                    if let Some(byte_offset) = bin_idx.get_line_offset(line_num - 1) {
                        let file_mmap = originblame::mmap_lines::mmap_file(&file)?
                            .ok_or_else(|| anyhow::anyhow!("File not found: {}", file.display()))?;
                        let data = &file_mmap[byte_offset as usize..];
                        let end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
                        std::str::from_utf8(&data[..end])
                            .map_err(|_| anyhow::anyhow!("Line {} is not valid UTF-8", line_num))?
                            .to_string()
                    } else {
                        eprintln!(
                        "hint: line offset table not found, run `ob index build` for O(1) blame"
                    );
                        let mmap = originblame::mmap_lines::mmap_file(&file)?
                            .ok_or_else(|| anyhow::anyhow!("File not found: {}", file.display()))?;
                        originblame::mmap_lines::iter_lines(&mmap)
                            .nth(line_num - 1)
                            .map(|bytes| std::str::from_utf8(bytes).unwrap().to_string())
                            .ok_or_else(|| anyhow::anyhow!("Line {} not found", line_num))?
                    }
                } else {
                    eprintln!("hint: no binary index found, run `ob index build` for O(1) blame");
                    let mmap = originblame::mmap_lines::mmap_file(&file)?
                        .ok_or_else(|| anyhow::anyhow!("File not found: {}", file.display()))?;
                    originblame::mmap_lines::iter_lines(&mmap)
                        .nth(line_num - 1)
                        .map(|bytes| std::str::from_utf8(bytes).unwrap().to_string())
                        .ok_or_else(|| anyhow::anyhow!("Line {} not found", line_num))?
                };

            let manifests =
                originblame::blame::blame(&dir, file.to_str().unwrap_or(""), &line_content)?;
            if manifests.is_empty() {
                println!("No provenance for line {}", line_num);
            } else {
                for m in manifests {
                    println!(
                        "line_hash={} file={} sources={:?}",
                        &m.line_hash[..12],
                        m.file,
                        m.sources
                    );
                }
            }
        }
        Commands::Show { author, email, section, license, revoked, tokenizer, index, dir, .. } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");

            if let Some(tok) = tokenizer {
                // Token-level query: prefer --section over --author/--email
                if let Some(sec) = section {
                    let result = originblame::show::show_by_section_token(&dir, &sec, &tok)?;
                    let entries = if revoked {
                        originblame::show::filter_token_entries_revoked(result.entries)
                    } else {
                        result.entries
                    };
                    let total_tokens: u64 = entries.iter().map(|e| e.token_count).sum();
                    let document_count = entries.len();
                    println!(
                        "{}: {} tokens across {} documents for section {} (tokenizer: {})",
                        dir.display(),
                        total_tokens,
                        document_count,
                        &sec[..12],
                        tok
                    );
                    for entry in &entries {
                        println!(
                            "  tokens={} sources={:?} revoked={}",
                            entry.token_count,
                            entry.sources,
                            entry.revoked
                        );
                    }
                    return Ok(());
                }

                let author_name = resolve_author(&dir, author.as_deref(), email.as_deref())?;
                let result = originblame::show::show_by_author_token(&dir, &author_name, &tok)?;
                let entries = if revoked {
                    originblame::show::filter_token_entries_revoked(result.entries)
                } else {
                    result.entries
                };
                let total_tokens: u64 = entries.iter().map(|e| e.token_count).sum();
                let document_count = entries.len();
                println!(
                    "{}: {} tokens across {} documents by {} (tokenizer: {})",
                    dir.display(),
                    total_tokens,
                    document_count,
                    author_name,
                    tok
                );
                for entry in &entries {
                    println!(
                        "  tokens={} sources={:?} revoked={}",
                        entry.token_count,
                        entry.sources,
                        entry.revoked
                    );
                }
            } else {
                // Document-level query
                let manifests = if let Some(sec) = &section {
                    originblame::show::show_by_section(&dir, sec)?
                } else if let Some(lic) = &license {
                    originblame::show::show_by_license(&dir, lic)?
                } else if author.is_some() || email.is_some() {
                    let author_name = resolve_author(&dir, author.as_deref(), email.as_deref())?;
                    let has_index = originblame::binary_index::BinaryIndex::open(&dir)?.is_some();
                    if index {
                        if !has_index {
                            eprintln!("hint: no binary index found, run `ob index build` for fast indexed queries");
                        }
                        originblame::show::show_by_author_indexed(&dir, &author_name)?
                    } else {
                        if !has_index {
                            eprintln!("hint: no binary index found, run `ob index build` for faster show/purge/blame");
                        }
                        originblame::show::show_by_author(&dir, &author_name)?
                    }
                } else {
                    originblame::show::show_all(&dir)?
                };

                let manifests = if revoked {
                    originblame::show::filter_documents_revoked(&dir, manifests)?
                } else {
                    originblame::show::filter_documents_active(&dir, manifests)?
                };

                let label = if let Some(sec) = &section {
                    format!("section {}", &sec[..12])
                } else if let Some(lic) = &license {
                    format!("license {}", lic)
                } else if author.is_some() || email.is_some() {
                    let author_name = resolve_author(&dir, author.as_deref(), email.as_deref())?;
                    format!("author {}", author_name)
                } else {
                    "all".to_string()
                };

                println!(
                    "{}: {} lines by {}",
                    dir.display(),
                    manifests.len(),
                    label
                );
                for m in manifests {
                    println!(
                        "  {} file={} sources={:?}",
                        &m.line_hash[..12],
                        m.file,
                        m.sources
                    );
                }
            }
        }
        Commands::Revoke { author, email, section, tokenizer, reverse, line_hash, file, dir } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let op_label = if reverse { "Restored" } else { "Revoked" };
            if let Some(lh) = line_hash {
                let f = file.ok_or_else(|| anyhow::anyhow!("--file is required with --line-hash"))?;
                anyhow::ensure!(author.is_none() && email.is_none() && tokenizer.is_none() && section.is_none(),
                    "--line-hash is mutually exclusive with --author, --email, --tokenizer, --section");
                let count = originblame::revoke::revoke_manifest(&dir, &lh, &f, reverse)?;
                println!("{} {} document records for line_hash={} file={}", op_label, count, &lh[..lh.len().min(12)], f);
            } else if let Some(sec_hash) = section {
                anyhow::ensure!(author.is_none() && email.is_none(), "--section is mutually exclusive with --author/--email");
                let count = originblame::revoke::revoke_section(&dir, &sec_hash, reverse)?;
                println!("{} {} section {}", op_label, count, &sec_hash[..8]);
            } else if let Some(tok) = tokenizer {
                let author_name = resolve_author(&dir, author.as_deref(), email.as_deref())?;
                let count = originblame::revoke::revoke_by_author_token(&dir, &author_name, &tok, reverse)?;
                println!("{} {} token-index entries for {} (tokenizer: {})", op_label, count, author_name, tok);
            } else {
                let author_name = resolve_author(&dir, author.as_deref(), email.as_deref())?;
                let count = originblame::revoke::revoke_by_author(&dir, &author_name, reverse)?;
                println!("{} {} document records for {}", op_label, count, author_name);
            }
        }
        Commands::Purge {
            file,
            index,
            author,
            email,
            dry_run,
            reverse,
            dir,
        } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            if reverse {
                anyhow::ensure!(!index, "--reverse is incompatible with --index");
                anyhow::ensure!(author.is_none() && email.is_none(), "--reverse is incompatible with --author/--email");
                anyhow::ensure!(!dry_run, "--reverse is incompatible with --dry-run");
                let count = originblame::purge::purge_reverse(&dir, &file)?;
                println!("Restored {} lines from purge archive", count);
            } else {
                let has_index = originblame::binary_index::BinaryIndex::open(&dir)?.is_some();
                let result = if index {
                    if !has_index {
                        eprintln!(
                            "hint: no binary index found, run `ob index build` for indexed purge"
                        );
                    }
                    let author_name = resolve_author(&dir, author.as_deref(), email.as_deref())?;
                    originblame::purge::purge_by_author_indexed(&dir, &author_name, &file, dry_run)?
                } else {
                    if !has_index {
                        eprintln!("hint: no binary index found, run `ob index build` for faster purge");
                    }
                    originblame::purge::purge_revoked(&dir, &file, dry_run)?
                };
            if dry_run {
                println!(
                    "Would purge {} lines, keep {} lines",
                    result.purged, result.kept
                );
            } else {
                println!("Purged {} lines, kept {} lines ({} archived)", result.purged, result.kept, result.archived);
            }
            }
        }
        Commands::Status { tokenizer, dir } => {
            let ob = originblame::ObDir::new(&dir);
            if !ob.is_initialized() {
                println!("Not initialized. Run `ob init` first.");
                return Ok(());
            }
            if let Some(tok) = tokenizer {
                let status = originblame::token_index::token_status(&dir, &tok)?;
                println!("Tokenizer: {}", tok);
                println!("Entries: {} ({} active, {} revoked)", status.total_entries, status.active_entries, status.revoked_entries);
                println!("Total tokens: {}", status.total_tokens);
            } else {
                let authors = originblame::authors::query(&dir, None, None)?;
                println!("Authors: {}", authors.len());
                let tokenizers = originblame::token_index::list_tokenizers(&dir)?;
                if !tokenizers.is_empty() {
                    println!("Tokenizers: {:?}", tokenizers);
                }
                println!("Path: {}", dir.display());
            }
        }
        Commands::Clean { split, dir } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let result = originblame::clean::clean(&dir, split)?;
            println!(
                "Merged {} document records from PID files",
                result.document_merged
            );
            println!("Deleted {} PID files", result.pid_files_deleted);
            if result.token_index_merged > 0 {
                println!("Merged {} token-index entries", result.token_index_merged);
                println!("Deleted {} token-index PID files", result.token_index_pid_deleted);
            }
            println!("Archived {} revoked records", result.archived_records);
            if result.log_rotated > 0 {
                println!("Rotated log file");
            }
        }
        Commands::Merge { absorb, dir } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let result = originblame::merge::absorb(&absorb, &dir)?;
            println!("Authors added: {}", result.authors_added);
            println!("Sections added: {}", result.sections_added);
            println!("Document records added: {}", result.document_added);
            if result.token_index_added > 0 {
                println!("Token-index added: {}", result.token_index_added);
            }
            println!("Skipped (duplicates/invalid): {}", result.skipped);
        }
        Commands::Index { command } => match command {
            IndexCommands::Build { dir } => {
                let ob = originblame::ObDir::new(&dir);
                anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
                let stats = originblame::index::build_index(&dir)?;
                println!(
                    "Index built: {} authors, {} sections, {} total entries, {} token-index entries",
                    stats.authors, stats.sections, stats.total, stats.token_index_entries
                );
            }
        },
        #[cfg(feature = "util")]
        Commands::ExportCopyright {
            output,
            data_file,
            dir,
        } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let result = originblame::export::export_copyright(
                &dir,
                output.as_deref(),
                data_file.as_deref(),
            )?;
            print!("{}", result);
        }
        #[cfg(feature = "util")]
        Commands::Reconcile {
            data_file,
            model,
            threshold,
            embedding_api,
            compute_all_embeddings,
            dir,
        } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let result = originblame::reconcile::reconcile(
                &dir,
                &data_file,
                model.as_deref(),
                threshold,
                embedding_api.as_deref(),
                compute_all_embeddings,
            )?;
            println!(
                "Reconcile complete: {} hash matched, {} new, {} orphans ({:.1}ms)",
                result.hash_matched, result.new_lines, result.orphans, result.duration_ms
            );
        }
        #[cfg(feature = "util")]
        Commands::Parse {
            parser,
            file,
            license,
            split,
            dir,
        } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let p = originblame::parsers::get_parser(&parser)
                .ok_or_else(|| anyhow::anyhow!("Unknown parser: {}", parser))?;
            let result = p.parse(&dir, &file, &license, split)?;
            println!(
                "Parsed {} pages, {} authors, {} sections, {} split files",
                result.pages_parsed,
                result.authors_registered,
                result.sections_created,
                result.split_files_created
            );
        }
        Commands::GenerateSet { tokenizer, output, dir } => {
            let ob = originblame::ObDir::new(&dir);
            anyhow::ensure!(ob.is_initialized(), ".ob/ not found. Run `ob init` first.");
            let bitmask = originblame::generate_set::generate_forget_set(&dir, &tokenizer)?;
            originblame::generate_set::write_forget_set(&output, &bitmask)?;
            let revoked_count = bitmask.iter().map(|b| b.count_ones() as usize).sum::<usize>();
            println!(
                "Forget set written to {} ({} bytes, {} revoked entries)",
                output.display(),
                bitmask.len(),
                revoked_count
            );
        }
        Commands::Version => {
            println!("ob {}", originblame::VERSION);
        }
    }

    Ok(())
}
