//! The CMU Pronouncing Dictionary, as genny bundles it: one line per word,
//! "word PH ON1 ES", the first variant of each. Unpacked once, on the first
//! lookup. Without the `cmudict` feature every lookup misses.

#[cfg(feature = "cmudict")]
pub fn lookup(word: &str) -> Option<&'static str> {
    use std::collections::HashMap;
    use std::io::Read;
    use std::sync::OnceLock;

    static PACKED: &[u8] = include_bytes!("../data/cmudict.txt.gz");
    static DICT: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut text = String::new();
        if let Err(err) = flate2::read::GzDecoder::new(PACKED).read_to_string(&mut text) {
            debug_assert!(false, "the bundled CMUdict unpacks: {err}");
            return HashMap::new();
        }
        // Kept for the life of the process, like the map that points into it.
        let text: &'static str = Box::leak(text.into_boxed_str());
        text.lines()
            .filter_map(|line| line.split_once(' '))
            .collect()
    })
    .get(word)
    .copied()
}

#[cfg(not(feature = "cmudict"))]
pub fn lookup(_word: &str) -> Option<&'static str> {
    None
}
