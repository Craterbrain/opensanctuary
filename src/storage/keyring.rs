use keyring::Entry;
use tracing::{debug, warn};

static TEST_KEYRING_FILE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Every secret goes to this JSON file instead of the OS keyring, from now on. For tests only: the
/// e2e test servers (the hidden `--test-keyring-file` flag, web/tests/e2e_helpers.ts) and
/// tests/core_tests use it so they never read, overwrite or litter the keyring of the machine
/// running them -- the same entries the real app uses (a plugin's API keys, the licensed-passage
/// key, the TLS key) would otherwise be shared. An explicit call, not an environment variable, so a
/// variable left in a shell or a service unit can never quietly move a real install's secrets into
/// a plain file. The first path wins.
pub fn use_test_keyring_file(path: std::path::PathBuf) {
    let _ = TEST_KEYRING_FILE.set(path);
}

fn test_keyring_file() -> Option<&'static std::path::Path> {
    TEST_KEYRING_FILE.get().map(|p| p.as_path())
}

/// The test keyring: a flat `{"service\u{1f}account": secret}` map, owner-only, replaced atomically
/// (written beside it, then renamed) so another process never reads half a file. Each read-modify-
/// write holds an exclusive lock on a `.lock` file beside it, so two servers sharing one test
/// keyring (a test that starts a second process on the same data) never lose each other's writes.
mod test_file {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    /// Held for one read-modify-write: this process's threads by the mutex, other processes by the
    /// file lock (released when the file closes).
    struct Held {
        _file: std::fs::File,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    fn hold(path: &Path) -> Result<Held, String> {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let lock_path = path.with_extension("lock");
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path).map_err(|e| format!("Test keyring lock error: {e}"))?;
        file.lock().map_err(|e| format!("Test keyring lock error: {e}"))?;
        Ok(Held { _file: file, _guard: guard })
    }

    fn key(service: &str, account: &str) -> String {
        format!("{service}\u{1f}{account}")
    }

    /// A missing file is an empty keyring; an unreadable or corrupt one is an error, never silently empty.
    fn load(path: &Path) -> Result<BTreeMap<String, String>, String> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("Test keyring {} is corrupt: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(format!("Test keyring read error: {e}")),
        }
    }

    fn save(path: &Path, map: &BTreeMap<String, String>) -> Result<(), String> {
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        let data = serde_json::to_vec(map).map_err(|e| e.to_string())?;
        crate::storage::write_private(&tmp, &data).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("Test keyring write error: {e}"))
    }

    pub fn get(path: &Path, service: &str, account: &str) -> Result<Option<String>, String> {
        let _held = hold(path)?;
        Ok(load(path)?.remove(&key(service, account)))
    }

    pub fn set(path: &Path, service: &str, account: &str, secret: &str) -> Result<(), String> {
        let _held = hold(path)?;
        let mut map = load(path)?;
        map.insert(key(service, account), secret.to_string());
        save(path, &map)
    }

    pub fn delete(path: &Path, service: &str, account: &str) -> Result<bool, String> {
        let _held = hold(path)?;
        let mut map = load(path)?;
        let existed = map.remove(&key(service, account)).is_some();
        if existed {
            save(path, &map)?;
        }
        Ok(existed)
    }
}

/// OS-Native Keyring Service providing secure, hardware-backed credential
/// storage across Windows (Windows Credential Manager) and Linux (FreeDesktop Secret Service).
pub struct KeyringService;

impl KeyringService {
    /// Stores an API key or password into the native OS vault.
    pub fn set_secret(service: &str, account: &str, secret: &str) -> Result<(), String> {
        debug!(service = %service, account = %account, "Writing secret to OS keyring");
        if let Some(path) = test_keyring_file() {
            return test_file::set(path, service, account, secret);
        }
        let entry = Entry::new(service, account).map_err(|e| format!("Keyring init error: {e}"))?;
        entry.set_password(secret).map_err(|e| format!("Keyring set_password error: {e}"))?;
        Ok(())
    }

    /// Retrieves an API key or password from the native OS vault.
    /// Returns `Ok(Some(secret))` if found, `Ok(None)` if no entry exists, or `Err(msg)` on OS error.
    pub fn get_secret(service: &str, account: &str) -> Result<Option<String>, String> {
        debug!(service = %service, account = %account, "Querying secret from OS keyring");
        if let Some(path) = test_keyring_file() {
            return test_file::get(path, service, account);
        }
        let entry = Entry::new(service, account).map_err(|e| format!("Keyring init error: {e}"))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => {
                warn!(service = %service, account = %account, error = %e, "Keyring read failed");
                Err(format!("Keyring get_password error: {e}"))
            }
        }
    }

    /// Checks if a secret exists in the OS vault without returning its contents.
    pub fn has_secret(service: &str, account: &str) -> Result<bool, String> {
        match Self::get_secret(service, account) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Deletes a secret from the native OS vault.
    /// Returns `Ok(true)` if deleted, `Ok(false)` if the entry did not exist.
    pub fn delete_secret(service: &str, account: &str) -> Result<bool, String> {
        debug!(service = %service, account = %account, "Deleting secret from OS keyring");
        if let Some(path) = test_keyring_file() {
            return test_file::delete(path, service, account);
        }
        let entry = Entry::new(service, account).map_err(|e| format!("Keyring init error: {e}"))?;
        match entry.delete_credential() {
            Ok(_) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => {
                warn!(service = %service, account = %account, error = %e, "Keyring delete failed");
                Err(format!("Keyring delete error: {e}"))
            }
        }
    }

    /// Computes a standard service namespace for a plugin.
    pub fn plugin_service(plugin_name: &str) -> String {
        format!("OpenSanctuary:Plugin:{}", plugin_name.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_keyring_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.json");
        assert_eq!(test_file::get(&path, "S", "a").unwrap(), None);
        test_file::set(&path, "S", "a", "one").unwrap();
        test_file::set(&path, "S", "b", "two").unwrap();
        test_file::set(&path, "S", "a", "three").unwrap();
        assert_eq!(test_file::get(&path, "S", "a").unwrap().as_deref(), Some("three"));
        assert_eq!(test_file::get(&path, "T", "a").unwrap(), None, "services are separate");
        assert!(test_file::delete(&path, "S", "a").unwrap());
        assert!(!test_file::delete(&path, "S", "a").unwrap());
        assert_eq!(test_file::get(&path, "S", "b").unwrap().as_deref(), Some("two"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A corrupt file is an error, not an empty keyring that a write would then overwrite.
        std::fs::write(&path, b"{not json").unwrap();
        assert!(test_file::get(&path, "S", "b").is_err());
        assert!(test_file::set(&path, "S", "c", "x").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[test]
    fn test_keyring_service_api_safety() {
        // Verifies that KeyringService handles non-existent queries safely without panics.
        let result = KeyringService::has_secret("OpenSanctuary:UnitTest", "nonexistent_account");
        match result {
            Ok(exists) => assert!(!exists),
            Err(e) => assert!(!e.is_empty()),
        }

        let get_result = KeyringService::get_secret("OpenSanctuary:UnitTest", "nonexistent_account");
        match get_result {
            Ok(val) => assert_eq!(val, None),
            Err(e) => assert!(!e.is_empty()),
        }
    }
}
