//! TLS Certificate Generation and HTTPS Support for OpenSanctuary.
//!
//! Provides automated self-signed X.509 certificate generation embedding Subject
//! Alternative Names (SANs) for localhost, 127.0.0.1, mDNS hostname, and all
//! discovered LAN network interfaces.

use std::collections::HashSet;
use std::net::IpAddr;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use crate::storage::{Database, KeyringService};
use super::NetworkInterfaceInfo;

pub const SETTING_KEY_TLS_ENABLED: &str = "httpsEnabled";
pub const SETTING_KEY_TLS_PORT: &str = "httpsPort";
pub const SETTING_KEY_TLS_CERT_PEM: &str = "tlsCertPem";
/// Legacy-only: the private key used to live under this settings key, which
/// `GET /api/settings` dumps wholesale to any unauthenticated caller. It's
/// only referenced now to detect and migrate an old deployment's key out of
/// the settings table.
const LEGACY_SETTING_KEY_TLS_KEY_PEM: &str = "tlsKeyPem";
/// The private key itself now lives in the OS keyring (Keychain / Credential
/// Manager / Secret Service) via `KeyringService`, the same password-manager
/// trust model used for plugin/provider secrets — never in the app's own
/// SQLite database, so a bug in a generic "dump all settings" route can't
/// leak it again structurally, not just by convention.
const TLS_KEYRING_SERVICE: &str = "OpenSanctuary:System";
/// Before keys were kept per install, every install on the machine shared this one keyring
/// entry (and `<config dir>/tls_private_key.pem` as its fallback), so a second install -- a
/// portable copy, a test server -- found a key that didn't match its certificate, minted a new
/// pair over it, and the first install did the same on its next start: each time breaking every
/// TV that pinned the old certificate. Now only read, to adopt a still-matching key once.
const LEGACY_SHARED_KEYRING_ACCOUNT: &str = "tls_private_key";
/// This install's id (in its own database, so it moves with the data): its keyring entry is
/// `tls_private_key:<id>`.
const SETTING_KEY_TLS_KEY_ID: &str = "tlsKeyId";
/// `OS_NEXT_TLS_KEY_STORE=file` keeps the private key in an owner-only file next to the database
/// instead of the OS keyring. The e2e test servers set it, so test runs never write to the
/// developer's real keyring.
const ENV_TLS_KEY_STORE: &str = "OS_NEXT_TLS_KEY_STORE";
const KEY_FILE_NAME: &str = "tls_private_key.pem";
pub const DEFAULT_HTTPS_PORT: u16 = 8443;

/// Where one install's TLS private key is kept.
#[derive(Debug, Clone)]
pub enum TlsKeyStore {
    /// The OS keyring, falling back to an owner-only file when no keyring is available
    /// (headless server, no Secret Service) -- without it the key would exist only in memory
    /// and every restart would mint a new certificate.
    Keyring { service: String, account: String, fallback: Option<std::path::PathBuf> },
    /// Only an owner-only file.
    File(std::path::PathBuf),
}

impl TlsKeyStore {
    /// This install's store: its own keyring entry (or the key file next to its database, if
    /// `OS_NEXT_TLS_KEY_STORE=file`). Creates the install's key id on first use.
    pub fn for_install(db: &Database) -> Self {
        let file = db.conn().path().filter(|p| !p.is_empty()).and_then(|p| std::path::Path::new(p).parent().map(|d| d.join(KEY_FILE_NAME)));
        if std::env::var(ENV_TLS_KEY_STORE).as_deref() == Ok("file") {
            if let Some(f) = file {
                return TlsKeyStore::File(f);
            }
        }
        let id = match db.get_setting(SETTING_KEY_TLS_KEY_ID).ok().flatten().filter(|v| !v.trim().is_empty()) {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().simple().to_string();
                let _ = db.set_setting(SETTING_KEY_TLS_KEY_ID, &id);
                id
            }
        };
        TlsKeyStore::Keyring { service: TLS_KEYRING_SERVICE.to_string(), account: format!("{LEGACY_SHARED_KEYRING_ACCOUNT}:{id}"), fallback: file }
    }

    fn read(&self) -> Option<String> {
        match self {
            TlsKeyStore::Keyring { service, account, fallback } => KeyringService::get_secret(service, account)
                .ok()
                .flatten()
                .filter(|k| !k.trim().is_empty())
                .or_else(|| fallback.as_deref().and_then(read_fallback_key_at)),
            TlsKeyStore::File(path) => read_fallback_key_at(path),
        }
    }

    fn write(&self, key_pem: &str) -> Result<(), String> {
        match self {
            TlsKeyStore::Keyring { service, account, fallback } => match KeyringService::set_secret(service, account, key_pem) {
                Ok(()) => {
                    if let Some(f) = fallback {
                        let _ = std::fs::remove_file(f);
                    }
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!("Failed to store TLS private key in OS keyring: {} — falling back to an owner-only key file", e);
                    let f = fallback.as_deref().ok_or_else(|| format!("no OS keyring ({e}) and no data folder for a key file"))?;
                    write_fallback_key_at(f, key_pem).map_err(|fe| fe.to_string())
                }
            },
            TlsKeyStore::File(path) => write_fallback_key_at(path, key_pem).map_err(|e| e.to_string()),
        }
    }
}

impl TlsKeyStore {
    /// The key every install used to share (see `LEGACY_SHARED_KEYRING_ACCOUNT`), looked up
    /// under this store's keyring service. A file-only store never had one.
    fn read_legacy_shared(&self) -> Option<String> {
        let TlsKeyStore::Keyring { service, .. } = self else { return None };
        KeyringService::get_secret(service, LEGACY_SHARED_KEYRING_ACCOUNT)
            .ok()
            .flatten()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                (service == TLS_KEYRING_SERVICE)
                    .then(|| read_fallback_key_at(&crate::storage::paths::config_dir().join(KEY_FILE_NAME)))
                    .flatten()
            })
    }
}

fn read_fallback_key_at(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok().filter(|k| !k.trim().is_empty())
}

fn write_fallback_key_at(path: &std::path::Path, key_pem: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        // Create with 0600 so the key is never briefly world-readable.
        let _ = std::fs::remove_file(path);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(key_pem.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, key_pem)
    }
}

/// Generates a self-signed X.509 certificate with Subject Alternative Names (SANs)
/// encompassing localhost, mDNS hostnames, and all active LAN IP addresses.
pub fn generate_self_signed_cert(
    hostname: &str,
    interfaces: &[NetworkInterfaceInfo],
) -> Result<(String, String), String> {
    let mut params = CertificateParams::default();

    // 1. Subject Distinguished Name
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "OpenSanctuary Presentation Server");
    dn.push(DnType::OrganizationName, "OpenSanctuary AV");
    params.distinguished_name = dn;

    // 2. Collect Unique SANs
    let mut dns_names = HashSet::new();
    let mut ip_addrs = HashSet::new();

    // Localhost identifiers
    dns_names.insert("localhost".to_string());
    ip_addrs.insert(IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)));
    ip_addrs.insert(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST));

    // Sanitized hostnames
    let clean_host = crate::network::sanitize_rfc1035_hostname(hostname);
    if !clean_host.is_empty() {
        dns_names.insert(format!("{}.local", clean_host));
        dns_names.insert(clean_host.clone());
    }
    dns_names.insert("opensanctuary.local".to_string());
    dns_names.insert("opensanctuary".to_string());

    // Discovered interface IPs
    for iface in interfaces {
        if iface.is_up && iface.enabled {
            if let Some(ref ip_str) = iface.ipv4 {
                if let Ok(ip) = ip_str.parse::<std::net::Ipv4Addr>() {
                    ip_addrs.insert(IpAddr::V4(ip));
                }
            }
        }
    }

    // Populate params.subject_alt_names
    for name in dns_names {
        params.subject_alt_names.push(SanType::DnsName(name.try_into().map_err(|e| format!("invalid SAN DNS name: {e}"))?));
    }
    for ip in ip_addrs {
        params.subject_alt_names.push(SanType::IpAddress(ip));
    }

    // 3. Generate keypair and self-sign
    let key_pair = KeyPair::generate().map_err(|e| format!("failed to generate RSA/ECDSA keypair: {e}"))?;
    let cert = params.self_signed(&key_pair).map_err(|e| format!("failed to self-sign certificate: {e}"))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    Ok((cert_pem, key_pem))
}

/// Retrieves this install's TLS certificate (settings table) and private key (its own keyring
/// entry), or generates and stores a fresh self-signed pair if there isn't a matching one.
pub fn get_or_create_tls_certificate(
    db: &Database,
    hostname: &str,
    interfaces: &[NetworkInterfaceInfo],
) -> Result<(String, String), String> {
    get_or_create_tls_certificate_in(db, hostname, interfaces, &TlsKeyStore::for_install(db))
}

/// `get_or_create_tls_certificate` with the key kept in `store`.
pub fn get_or_create_tls_certificate_in(
    db: &Database,
    hostname: &str,
    interfaces: &[NetworkInterfaceInfo],
    store: &TlsKeyStore,
) -> Result<(String, String), String> {
    let cert = db.get_setting(SETTING_KEY_TLS_CERT_PEM).ok().flatten().filter(|c| !c.trim().is_empty());
    let stored_key = store.read();

    if let (Some(cert), Some(key)) = (&cert, &stored_key) {
        if validate_tls_keypair(cert, key) {
            return Ok((cert.clone(), key.clone()));
        } else {
            tracing::warn!("Stored TLS certificate and private key do not match (KeyMismatch) — regenerating consistent pair");
        }
    }

    // One-time move from the entry every install used to share: keep the key only if it is
    // still this install's (matches its certificate), so already-paired TVs keep working. The
    // shared entry is left alone -- an older build of another install may still be using it.
    if let (Some(cert), None) = (&cert, &stored_key) {
        if let Some(shared) = store.read_legacy_shared().filter(|k| validate_tls_keypair(cert, k)) {
            if store.write(&shared).is_ok() {
                tracing::info!("Moved this install's TLS private key into its own keyring entry");
                return Ok((cert.clone(), shared));
            }
        }
    }

    // One-time migration: an older build stored the private key in the
    // plaintext settings table, where GET /api/settings dumps it to any
    // unauthenticated caller. If a cert already exists but the keyring
    // doesn't have a key yet, move a legacy key into the keyring instead of
    // needlessly generating (and having every already-paired client
    // distrust) a brand new cert.
    if let Some(cert) = &cert {
        if let Ok(Some(legacy_key)) = db.get_setting(LEGACY_SETTING_KEY_TLS_KEY_PEM) {
            if !legacy_key.trim().is_empty() {
                if validate_tls_keypair(cert, &legacy_key) {
                    if store.write(&legacy_key).is_ok() {
                        let _ = db.delete_setting(LEGACY_SETTING_KEY_TLS_KEY_PEM);
                        tracing::info!("Migrated TLS private key out of the settings table into the OS keyring");
                        return Ok((cert.clone(), legacy_key));
                    }
                } else {
                    let _ = db.delete_setting(LEGACY_SETTING_KEY_TLS_KEY_PEM);
                }
            }
        }
    }

    // Generate fresh certificate and store
    regenerate_tls_certificate_in(db, hostname, interfaces, store)
}

/// SHA-256 fingerprint of a cert's raw DER bytes, as lowercase hex -- used
/// to let the Android TV app verify it's actually talking to *this*
/// console's certificate rather than trusting any self-signed cert handed
/// to it (`src/network/adb.rs::provision`'s `cert_fingerprint` intent
/// extra; `PairingManager.kt` pins and checks it on every connection). Not
/// a secret -- this is exactly what a human would read off a padlock
/// icon's "certificate details," just delivered through an already-trusted
/// channel (the ADB launch, or the TV's own screen) instead of asking the
/// operator to compare hex strings by eye.
pub fn cert_fingerprint_sha256(cert_pem: &str) -> Result<String, String> {
    let certs_res: Result<Vec<rustls::pki_types::CertificateDer>, _> =
        rustls_pemfile::certs(&mut cert_pem.as_bytes()).collect();
    let certs = certs_res.map_err(|e| format!("Failed to parse certificate PEM: {e}"))?;
    let cert = certs.first().ok_or("No certificate found in PEM")?;

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(cert.as_ref());
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Validates whether a given certificate and private key PEM match and form a valid Rustls configuration.
pub fn validate_tls_keypair(cert_pem: &str, key_pem: &str) -> bool {
    let certs_res: Result<Vec<rustls::pki_types::CertificateDer>, _> =
        rustls_pemfile::certs(&mut cert_pem.as_bytes()).collect();
    let certs = match certs_res {
        Ok(c) if !c.is_empty() => c,
        _ => return false,
    };

    let key = match rustls_pemfile::private_key(&mut key_pem.as_bytes()) {
        Ok(Some(k)) => k,
        _ => return false,
    };

    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    if let Ok(builder) = rustls::ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions() {
        builder.with_no_client_auth().with_single_cert(certs, key).is_ok()
    } else {
        false
    }
}

/// Regenerates a fresh self-signed TLS certificate with updated SANs and persists it —
/// the certificate (public by nature) in the settings table, the private key in the
/// OS keyring, never the app's own database.
pub fn regenerate_tls_certificate(
    db: &Database,
    hostname: &str,
    interfaces: &[NetworkInterfaceInfo],
) -> Result<(String, String), String> {
    regenerate_tls_certificate_in(db, hostname, interfaces, &TlsKeyStore::for_install(db))
}

/// `regenerate_tls_certificate` with the key kept in `store`.
pub fn regenerate_tls_certificate_in(
    db: &Database,
    hostname: &str,
    interfaces: &[NetworkInterfaceInfo],
    store: &TlsKeyStore,
) -> Result<(String, String), String> {
    let (cert_pem, key_pem) = generate_self_signed_cert(hostname, interfaces)?;
    let _ = db.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert_pem);
    if let Err(e) = store.write(&key_pem) {
        tracing::error!("Failed to store TLS private key: {} — HTTPS will regenerate a new cert every restart until this is resolved", e);
    }
    // Clear any stale plaintext copy from a pre-migration deployment.
    let _ = db.delete_setting(LEGACY_SETTING_KEY_TLS_KEY_PEM);
    tracing::info!("Regenerated self-signed TLS certificate with updated interface SANs");
    Ok((cert_pem, key_pem))
}

/// Converts PEM certificate and key bytes into an Axum-Server RustlsConfig.
pub async fn create_rustls_config(
    cert_pem: &str,
    key_pem: &str,
) -> Result<axum_server::tls_rustls::RustlsConfig, String> {
    axum_server::tls_rustls::RustlsConfig::from_pem(
        cert_pem.as_bytes().to_vec(),
        key_pem.as_bytes().to_vec(),
    )
    .await
    .map_err(|e| format!("failed to build RustlsConfig from PEM: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_fingerprint_is_stable_and_changes_with_a_different_cert() {
        let (cert_a, _key_a) = generate_self_signed_cert("test-sanctuary", &[]).unwrap();
        let (cert_b, _key_b) = generate_self_signed_cert("test-sanctuary", &[]).unwrap();

        let fp_a1 = cert_fingerprint_sha256(&cert_a).unwrap();
        let fp_a2 = cert_fingerprint_sha256(&cert_a).unwrap();
        assert_eq!(fp_a1, fp_a2, "hashing the same cert twice must be deterministic");
        assert_eq!(fp_a1.len(), 64, "sha256 hex should be 64 chars");
        assert!(fp_a1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));

        let fp_b = cert_fingerprint_sha256(&cert_b).unwrap();
        assert_ne!(fp_a1, fp_b, "two independently generated certs must not collide");
    }

    #[test]
    fn cert_fingerprint_rejects_garbage_input() {
        assert!(cert_fingerprint_sha256("not a certificate").is_err());
    }

    // Exercises the owner-only file fallback directly against a temp path,
    // independent of whether this machine/CI runner has a real OS keyring --
    // this is specifically the headless/no-Secret-Service case the fallback
    // exists for (see `fallback_key_path`'s doc comment), so it must not be
    // skipped the way `test_tls_private_key_storage_and_legacy_migration`
    // skips without a working keyring.
    #[test]
    fn fallback_key_file_round_trips_without_a_keyring() {
        let test_dir = tempfile::tempdir().unwrap();
        let path = test_dir.path().join("nested").join("tls_private_key.pem");

        assert!(read_fallback_key_at(&path).is_none(), "nothing written yet");

        let (_cert, key_pem) = generate_self_signed_cert("fallback-host", &[]).unwrap();
        write_fallback_key_at(&path, &key_pem).unwrap();
        assert_eq!(read_fallback_key_at(&path).as_deref(), Some(key_pem.as_str()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "private key file must never be group/world readable");
        }

        // A later restart that regenerates (e.g. after a KeyMismatch) must be
        // able to overwrite the file, not fail with AlreadyExists.
        let (_cert2, key_pem2) = generate_self_signed_cert("fallback-host-2", &[]).unwrap();
        write_fallback_key_at(&path, &key_pem2).unwrap();
        assert_eq!(read_fallback_key_at(&path).as_deref(), Some(key_pem2.as_str()));
    }

    #[test]
    fn fallback_key_read_ignores_a_blank_file() {
        let test_dir = tempfile::tempdir().unwrap();
        let path = test_dir.path().join("tls_private_key.pem");
        std::fs::write(&path, "   \n").unwrap();
        assert!(read_fallback_key_at(&path).is_none(), "whitespace-only content must not be treated as a key");
    }

    fn temp_db(dir: &tempfile::TempDir, name: &str) -> Database {
        Database::new(dir.path().join(name).to_str().unwrap()).unwrap()
    }

    fn file_store(dir: &tempfile::TempDir, name: &str) -> TlsKeyStore {
        TlsKeyStore::File(dir.path().join(name))
    }

    #[test]
    fn a_stored_pair_is_returned_again_and_the_key_never_lands_in_settings() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir, "a.db");
        let store = file_store(&dir, "key.pem");

        let (cert1, key1) = get_or_create_tls_certificate_in(&db, "testhost", &[], &store).unwrap();
        assert!(validate_tls_keypair(&cert1, &key1));
        let settings = db.get_settings().unwrap();
        // `GET /api/settings` dumps the settings table to anyone; the key must never be there.
        assert!(!settings.contains_key(LEGACY_SETTING_KEY_TLS_KEY_PEM));
        assert!(!settings.values().any(|v| v.contains("PRIVATE KEY")));
        assert_eq!(settings.get(SETTING_KEY_TLS_CERT_PEM).unwrap(), &cert1);

        let (cert2, key2) = get_or_create_tls_certificate_in(&db, "testhost", &[], &store).unwrap();
        assert_eq!((cert1, key1), (cert2, key2), "a restart must keep the certificate TVs have pinned");
    }

    /// The bug this layout fixes: two installs on one machine used to share one key entry, so
    /// each start of one minted a new certificate over the other's (breaking pinned TVs).
    #[test]
    fn two_installs_keep_their_own_certificates() {
        let dir = tempfile::tempdir().unwrap();
        let (db_a, db_b) = (temp_db(&dir, "a.db"), temp_db(&dir, "b.db"));
        let (store_a, store_b) = (file_store(&dir, "a.pem"), file_store(&dir, "b.pem"));

        let (cert_a, _) = get_or_create_tls_certificate_in(&db_a, "a", &[], &store_a).unwrap();
        let (cert_b, _) = get_or_create_tls_certificate_in(&db_b, "b", &[], &store_b).unwrap();
        assert_ne!(cert_a, cert_b);
        assert_eq!(get_or_create_tls_certificate_in(&db_a, "a", &[], &store_a).unwrap().0, cert_a);
        assert_eq!(get_or_create_tls_certificate_in(&db_b, "b", &[], &store_b).unwrap().0, cert_b);
    }

    #[test]
    fn each_install_gets_its_own_keyring_entry_and_keeps_it() {
        if std::env::var(ENV_TLS_KEY_STORE).is_ok() {
            return; // this test is about the default (keyring) layout
        }
        let dir = tempfile::tempdir().unwrap();
        let (db_a, db_b) = (temp_db(&dir, "a.db"), temp_db(&dir, "b.db"));
        let account = |db: &Database| match TlsKeyStore::for_install(db) {
            TlsKeyStore::Keyring { service, account, fallback } => {
                assert_eq!(service, TLS_KEYRING_SERVICE);
                // The no-keyring fallback sits with this install's data, not in a shared folder.
                assert_eq!(fallback, Some(dir.path().join(KEY_FILE_NAME)));
                account
            }
            other => panic!("expected the keyring store, got {other:?}"),
        };
        let a = account(&db_a);
        assert!(a.starts_with("tls_private_key:") && a.len() > "tls_private_key:".len());
        assert_eq!(account(&db_a), a, "the id is stored in the install's database and reused");
        assert_ne!(account(&db_b), a);
    }

    #[test]
    fn a_key_that_does_not_match_the_certificate_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir, "a.db");
        let store = file_store(&dir, "key.pem");
        let (cert, _) = generate_self_signed_cert("host_a", &[]).unwrap();
        let (_, other_key) = generate_self_signed_cert("host_b", &[]).unwrap();
        db.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert).unwrap();
        store.write(&other_key).unwrap();

        let (fixed_cert, fixed_key) = get_or_create_tls_certificate_in(&db, "host_a", &[], &store).unwrap();
        assert!(validate_tls_keypair(&fixed_cert, &fixed_key));
        assert_ne!(fixed_cert, cert);
        assert_eq!(store.read().as_deref(), Some(fixed_key.as_str()));
    }

    #[test]
    fn a_key_left_in_the_settings_table_by_an_old_build_is_moved_out_keeping_the_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir, "a.db");
        let store = file_store(&dir, "key.pem");
        let (cert, key) = generate_self_signed_cert("legacyhost", &[]).unwrap();
        db.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert).unwrap();
        db.set_setting(LEGACY_SETTING_KEY_TLS_KEY_PEM, &key).unwrap();

        assert_eq!(get_or_create_tls_certificate_in(&db, "legacyhost", &[], &store).unwrap(), (cert, key.clone()));
        assert!(!db.get_settings().unwrap().contains_key(LEGACY_SETTING_KEY_TLS_KEY_PEM));
        assert_eq!(store.read(), Some(key));
    }

    /// Upgrading from the shared entry: an install adopts the shared key only if it is still
    /// its own (matches its certificate), and leaves the shared entry for older builds. Uses a
    /// test keyring service, never the app's own entries.
    #[test]
    fn upgrading_adopts_the_shared_key_only_when_it_is_this_installs() {
        let service = format!("OpenSanctuary:Test:{}", uuid::Uuid::new_v4().simple());
        if KeyringService::set_secret(&service, "probe", "probe").is_err()
            || !matches!(KeyringService::get_secret(&service, "probe"), Ok(Some(v)) if v == "probe")
        {
            eprintln!("skipping: no working OS keyring in this environment");
            return;
        }
        let _ = KeyringService::delete_secret(&service, "probe");
        let dir = tempfile::tempdir().unwrap();
        let store = |account: &str| TlsKeyStore::Keyring { service: service.clone(), account: account.to_string(), fallback: None };

        // Its own key in the shared entry: kept, so paired TVs keep working.
        let db = temp_db(&dir, "a.db");
        let (cert, key) = generate_self_signed_cert("upgrade", &[]).unwrap();
        db.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert).unwrap();
        KeyringService::set_secret(&service, LEGACY_SHARED_KEYRING_ACCOUNT, &key).unwrap();
        assert_eq!(get_or_create_tls_certificate_in(&db, "upgrade", &[], &store("tls_private_key:a")).unwrap(), (cert, key.clone()));
        assert_eq!(KeyringService::get_secret(&service, "tls_private_key:a").unwrap().as_deref(), Some(key.as_str()));
        assert_eq!(KeyringService::get_secret(&service, LEGACY_SHARED_KEYRING_ACCOUNT).unwrap().as_deref(), Some(key.as_str()), "the shared entry is left alone");

        // Another install's key there: not adopted, and not overwritten either.
        let db_b = temp_db(&dir, "b.db");
        let (cert_b, _) = generate_self_signed_cert("other", &[]).unwrap();
        db_b.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert_b).unwrap();
        let (new_cert, new_key) = get_or_create_tls_certificate_in(&db_b, "other", &[], &store("tls_private_key:b")).unwrap();
        assert_ne!(new_cert, cert_b);
        assert_ne!(new_key, key);
        assert_eq!(KeyringService::get_secret(&service, LEGACY_SHARED_KEYRING_ACCOUNT).unwrap().as_deref(), Some(key.as_str()));

        for account in [LEGACY_SHARED_KEYRING_ACCOUNT, "tls_private_key:a", "tls_private_key:b"] {
            let _ = KeyringService::delete_secret(&service, account);
        }
    }
}
