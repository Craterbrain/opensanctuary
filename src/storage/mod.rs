pub mod bible_import;
pub mod bundle;
pub mod db;
pub mod emf;
pub mod ewsx;
pub mod freeshow_import;
pub mod freeshow_show_import;
pub mod media_sniff;
pub mod openlp_import;
pub mod paths;
pub mod pattern_cache;
pub mod pd_songs;
pub mod pptx_geom;
pub mod pptx_import;
pub mod pptx_inherit;
pub mod wmf;
pub mod text_metrics;
pub mod xml_scan;
pub mod odp_chart;
pub mod odp_geom;
pub mod odp_import;
pub mod odp_export;
pub mod rtf;
pub mod transcode;
pub mod keyring;

/// Writes a file only its owner can read (0600 on Unix), for keys and secrets kept on disk.
#[cfg(unix)]
pub(crate) fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(data)
}

#[cfg(not(unix))]
pub(crate) fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

pub use db::{CcliUsageRow, Database, InstalledBibleInfo, PairedDevice};
pub use ewsx::EwsxManager;
pub use freeshow_import::{find_book_meta, find_book_meta_exact, parse_scripture_reference, BibleBookMeta, FreeShowImporter, ParsedScriptureRef, BIBLE_BOOKS_META, KNOWN_BIBLE_VERSIONS};
pub use freeshow_show_import::FreeShowShowImporter;
pub use keyring::KeyringService;
pub use openlp_import::OpenLPImporter;
pub use paths::{DataDirSource, InstallConfig, ResolvedPaths};
pub use pptx_import::PptxImporter;
pub use odp_import::OdpImporter;
pub use odp_export::OdpExporter;
