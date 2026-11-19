use std::sync::OnceLock;

pub const SEMVER: &str = env!("CARGO_PKG_VERSION");

pub fn display() -> &'static str {
    static DISPLAY: OnceLock<String> = OnceLock::new();
    DISPLAY.get_or_init(|| SEMVER.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_the_cargo_semver() {
        assert_eq!(display(), "1.5.0");
    }
}
