pub fn version() -> &'static str {
    "zerocast-core 0.1.0"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string() {
        assert!(version().contains("zerocast-core"));
    }
}
