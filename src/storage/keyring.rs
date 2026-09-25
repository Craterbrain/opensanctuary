use keyring::Entry;
use tracing::{debug, warn};

/// OS-Native Keyring Service providing secure, hardware-backed credential
/// storage across Windows (Windows Credential Manager) and Linux (FreeDesktop Secret Service).
pub struct KeyringService;

impl KeyringService {
    /// Stores an API key or password into the native OS vault.
    pub fn set_secret(service: &str, account: &str, secret: &str) -> Result<(), String> {
        debug!(service = %service, account = %account, "Writing secret to OS keyring");
        let entry = Entry::new(service, account).map_err(|e| format!("Keyring init error: {e}"))?;
        entry.set_password(secret).map_err(|e| format!("Keyring set_password error: {e}"))?;
        Ok(())
    }

    /// Retrieves an API key or password from the native OS vault.
    /// Returns `Ok(Some(secret))` if found, `Ok(None)` if no entry exists, or `Err(msg)` on OS error.
    pub fn get_secret(service: &str, account: &str) -> Result<Option<String>, String> {
        debug!(service = %service, account = %account, "Querying secret from OS keyring");
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

    /// Computes a standard service namespace for a media or external provider.
    pub fn provider_service(provider_name: &str) -> String {
        format!("OpenSanctuary:Provider:{}", provider_name.trim())
    }

    /// Retrieves a secret stored under a plugin's vault.
    pub fn get_plugin_secret(plugin_name: &str, account: &str) -> Result<Option<String>, String> {
        Self::get_secret(&Self::plugin_service(plugin_name), account)
    }

    /// Sets a secret stored under a plugin's vault.
    pub fn set_plugin_secret(plugin_name: &str, account: &str, secret: &str) -> Result<(), String> {
        Self::set_secret(&Self::plugin_service(plugin_name), account, secret)
    }

    /// Deletes a secret stored under a plugin's vault.
    pub fn delete_plugin_secret(plugin_name: &str, account: &str) -> Result<bool, String> {
        Self::delete_secret(&Self::plugin_service(plugin_name), account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
