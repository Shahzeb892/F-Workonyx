/// Macro that helps to check test file exist at compile time.
/// [link](https://stackoverflow.com/questions/30003921/how-can-i-locate-resources-for-testing-with-cargo)
/// [link](https://stackoverflow.com/questions/73187970/compile-time-check-if-file-at-path-exists-like-include-str)
#[macro_export]
macro_rules! test_file_path {
    ($arg1:expr) => {{
        // TODO: opportunity for improvement
        let _ = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), $arg1));
        let r = concat!(env!("CARGO_MANIFEST_DIR"), $arg1);
        r
    }};
}

/// Macro to test if directories exist.
#[macro_export]
macro_rules! test_dirs_path {
    ($arg1:expr) => {{
        //TODO: opportunity for improvement
        let _ = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), $arg1));
        let r = concat!(env!("CARGO_MANIFEST_DIR"), $arg1);
        r
    }};
}
