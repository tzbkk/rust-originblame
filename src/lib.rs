pub mod authors;
pub mod binary_index;
pub mod blame;
pub mod clean;
pub mod hash;
pub mod index;
pub mod indexer;
pub mod merge;
pub mod mmap_lines;
pub mod oplog;
pub mod purge;
pub mod revoke;
pub mod register;
pub mod show;
pub mod storage;
pub mod generate_set;
pub mod token_bin_index;
pub mod token_index;
pub mod track;

#[cfg(feature = "util")]
pub mod embeddings;
#[cfg(feature = "util")]
pub mod export;
#[cfg(feature = "util")]
pub mod parsers;
#[cfg(feature = "util")]
pub mod reconcile;

#[cfg(feature = "python")]
mod python;

use anyhow::Result;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct ObDir {
    pub root: std::path::PathBuf,
}

impl ObDir {
    pub fn new(root: &std::path::Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub fn ob_path(&self) -> std::path::PathBuf {
        self.root.join(".ob")
    }

    pub fn is_initialized(&self) -> bool {
        self.ob_path().is_dir()
    }

    pub fn init(&self) -> Result<()> {
        let ob = self.ob_path();
        std::fs::create_dir_all(ob.join("authors"))?;
        std::fs::create_dir_all(ob.join("sections"))?;
        std::fs::create_dir_all(ob.join("document-index"))?;
        std::fs::create_dir_all(ob.join("index"))?;
        std::fs::write(ob.join("log"), "")?;
        Ok(())
    }
}
