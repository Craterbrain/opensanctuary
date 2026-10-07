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
const TLS_KEYRING_ACCOUNT: &str = "tls_private_key";
pub const DEFAULT_HTTPS_PORT: u16 = 8443;

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

/// Retrieves stored TLS certificate and key from SQLite settings, or generates
/// and stores a fresh self-signed certificate if none exists.
pub fn get_or_create_tls_certificate(
    db: &Database,
    hostname: &str,
    interfaces: &[NetworkInterfaceInfo],
) -> Result<(String, String), String> {
    let cert = db.get_setting(SETTING_KEY_TLS_CERT_PEM).ok().flatten().filter(|c| !c.trim().is_empty());
    let keyring_key = KeyringService::get_secret(TLS_KEYRING_SERVICE, TLS_KEYRING_ACCOUNT)
        .ok()
        .flatten()
        .filter(|k| !k.trim().is_empty());

    if let (Some(cert), Some(key)) = (&cert, &keyring_key) {
        if validate_tls_keypair(cert, key) {
            return Ok((cert.clone(), key.clone()));
        } else {
            tracing::warn!("Stored TLS certificate and OS keyring private key do not match (KeyMismatch) — regenerating consistent pair");
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
                    if KeyringService::set_secret(TLS_KEYRING_SERVICE, TLS_KEYRING_ACCOUNT, &legacy_key).is_ok() {
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
    regenerate_tls_certificate(db, hostname, interfaces)
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
    let (cert_pem, key_pem) = generate_self_signed_cert(hostname, interfaces)?;
    let _ = db.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert_pem);
    if let Err(e) = KeyringService::set_secret(TLS_KEYRING_SERVICE, TLS_KEYRING_ACCOUNT, &key_pem) {
        tracing::error!("Failed to store TLS private key in OS keyring: {} — HTTPS will regenerate a new cert every restart until this is resolved", e);
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

    // Both scenarios below share the same real OS keyring entry (it's
    // process/machine-wide state, not scoped per-database the way SQLite
    // is) — combined into one test function so `cargo test`'s default
    // parallelism across functions can't interleave their writes to it and
    // produce a false failure. Each still asserts its own thing.
    #[test]
    fn test_tls_private_key_storage_and_legacy_migration() {
        // 1. Regression test: the private key used to be stored under
        // `tlsKeyPem` in the plain settings table, which `GET /api/settings`
        // dumps wholesale to any unauthenticated caller. It must never land
        // there again, and a second call must return the SAME cert/key
        // rather than regenerating — proving the keyring-backed key
        // round-trips.
        let test_dir = tempfile::tempdir().unwrap();
        let db_path = test_dir.path().join("test_tls.db");
        let db = Database::new(db_path.to_str().unwrap()).unwrap();

        let (cert1, key1) = get_or_create_tls_certificate(&db, "testhost", &[]).unwrap();
        assert!(!cert1.is_empty());
        assert!(!key1.is_empty());

        let settings = db.get_settings().unwrap();
        assert!(settings.get(LEGACY_SETTING_KEY_TLS_KEY_PEM).is_none());
        assert_eq!(settings.get(SETTING_KEY_TLS_CERT_PEM).unwrap(), &cert1);

        let (cert2, key2) = get_or_create_tls_certificate(&db, "testhost", &[]).unwrap();
        assert_eq!(cert1, cert2);
        assert_eq!(key1, key2);

        // 2. An older deployment that still has the key sitting in the
        // plaintext settings table (pre-fix) gets it migrated into the
        // keyring on next read, keeping the same already-trusted cert
        // rather than silently generating a new one (which would break any
        // client that pinned it). Clear the entry part 1 just wrote first —
        // the keyring is real, process-wide OS state (unlike the SQLite db,
        // it isn't isolated per scenario), so without this the lookup below
        // would just find part 1's key instead of actually exercising the
        // migration path.
        let _ = KeyringService::delete_secret(TLS_KEYRING_SERVICE, TLS_KEYRING_ACCOUNT);

        let test_dir2 = tempfile::tempdir().unwrap();
        let db_path2 = test_dir2.path().join("test_tls_migrate.db");
        let db2 = Database::new(db_path2.to_str().unwrap()).unwrap();

        let (cert_pem, key_pem) = generate_self_signed_cert("legacyhost", &[]).unwrap();
        db2.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert_pem).unwrap();
        db2.set_setting(LEGACY_SETTING_KEY_TLS_KEY_PEM, &key_pem).unwrap();

        let (migrated_cert, migrated_key) = get_or_create_tls_certificate(&db2, "legacyhost", &[]).unwrap();
        assert_eq!(migrated_cert, cert_pem);
        assert_eq!(migrated_key, key_pem);

        let settings2 = db2.get_settings().unwrap();
        assert!(settings2.get(LEGACY_SETTING_KEY_TLS_KEY_PEM).is_none());

        // 3. KeyMismatch detection and auto-regeneration: if the database has
        // an existing certificate, but the keyring has a different key (e.g. from
        // another machine profile, restored db, or test run), get_or_create_tls_certificate
        // must detect the mismatch, regenerate a matching keypair, and succeed.
        let (cert_mismatched, _) = generate_self_signed_cert("host_a", &[]).unwrap();
        let (_, key_mismatched) = generate_self_signed_cert("host_b", &[]).unwrap();
        db2.set_setting(SETTING_KEY_TLS_CERT_PEM, &cert_mismatched).unwrap();
        let _ = KeyringService::set_secret(TLS_KEYRING_SERVICE, TLS_KEYRING_ACCOUNT, &key_mismatched);

        assert!(!validate_tls_keypair(&cert_mismatched, &key_mismatched));

        let (fixed_cert, fixed_key) = get_or_create_tls_certificate(&db2, "legacyhost", &[]).unwrap();
        assert!(validate_tls_keypair(&fixed_cert, &fixed_key));
        assert_ne!(fixed_cert, cert_mismatched);

        // Don't leave a test entry behind in the developer's real OS keyring.
        let _ = KeyringService::delete_secret(TLS_KEYRING_SERVICE, TLS_KEYRING_ACCOUNT);
    }
}
