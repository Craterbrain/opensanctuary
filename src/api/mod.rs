pub mod error;
pub mod routes;
pub mod ws;

pub use error::AppError;
pub use routes::*;
pub use ws::*;

/// Constant-time string equality for secrets (host/session tokens): the time taken
/// doesn't depend on how many leading bytes match. Length is not hidden, which is
/// fine for fixed-format random tokens.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod ct_eq_tests {
    use super::ct_eq;

    #[test]
    fn compares_like_equality() {
        assert!(ct_eq("abc-123", "abc-123"));
        assert!(!ct_eq("abc-123", "abc-124"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(ct_eq("", ""));
    }
}
