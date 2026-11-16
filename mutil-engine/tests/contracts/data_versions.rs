//! The Unicode data the engine is built on (SCO-4, SCO-8): the lock file pins
//! the ICU4X data crates to the release whose data is the version the crate
//! reports, so a dependency update cannot change the data silently.

use mutil_engine::UNICODE_DATA_VERSION;

const LOCK: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));

/// Every version `Cargo.lock` records for the package `name`.
fn locked_versions(name: &str) -> Vec<&'static str> {
    let wanted = format!("name = \"{name}\"");
    let mut lines = LOCK.lines();
    let mut versions = Vec::new();
    while let Some(line) = lines.next() {
        if line == wanted {
            let version = lines
                .next()
                .and_then(|next| next.strip_prefix("version = \""))
                .and_then(|rest| rest.strip_suffix('"'))
                .expect("a package entry states its version after its name");
            versions.push(version);
        }
    }
    versions
}

#[test]
fn the_locked_icu_data_is_the_reported_unicode_version() {
    // ICU4X 2.4.x data is generated from Unicode 18.0.0.
    assert_eq!(UNICODE_DATA_VERSION, "18.0.0");
    for data_crate in [
        "icu_segmenter_data",
        "icu_properties_data",
        "icu_normalizer_data",
    ] {
        let versions = locked_versions(data_crate);
        assert_eq!(
            versions.len(),
            1,
            "{data_crate} is locked once: {versions:?}"
        );
        assert!(
            versions[0].starts_with("2.4."),
            "{data_crate} {} is not a 2.4.x release: UNICODE_DATA_VERSION must follow its data",
            versions[0]
        );
    }
}

#[test]
fn the_lock_file_reader_finds_packages() {
    assert_eq!(locked_versions("mutil-engine"), ["0.0.1"]);
    assert!(locked_versions("no-such-package").is_empty());
}
