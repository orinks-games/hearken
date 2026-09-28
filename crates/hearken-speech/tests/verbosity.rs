use hearken_speech::{Category, Level, NoCategory, Rule, SpeechSettings, Verbosity};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum C {
    Safety,
    Chatter,
}

impl Category for C {
    fn all() -> &'static [Self] {
        &[C::Safety, C::Chatter]
    }
    fn id(self) -> &'static str {
        match self {
            C::Safety => "safety",
            C::Chatter => "chatter",
        }
    }
}

static LEVELS: [Level<C>; 3] = [
    Level {
        id: "full",
        label: "Full",
        rules: &[(C::Safety, Rule::Full), (C::Chatter, Rule::Full)],
    },
    Level {
        id: "quiet",
        label: "Quiet",
        rules: &[
            (C::Safety, Rule::Full),
            (C::Chatter, Rule::Earcon("ui/tick")),
        ],
    },
    Level {
        id: "odd",
        label: "Odd",
        rules: &[(C::Safety, Rule::Terse), (C::Safety, Rule::Silent)],
    },
];

fn table() -> Verbosity<C> {
    Verbosity { levels: &LEVELS }
}

#[test]
fn a_level_says_what_it_does_with_each_category() {
    assert_eq!(table().rule("quiet", C::Chatter), Rule::Earcon("ui/tick"));
    assert_eq!(table().rule("quiet", C::Safety), Rule::Full);
}

#[test]
fn an_unknown_level_is_the_first_and_an_unlisted_category_is_full() {
    assert_eq!(table().level("gone").map(|l| l.id), Some("full"));
    assert_eq!(table().level("").map(|l| l.id), Some("full"));
    assert_eq!(table().rule("odd", C::Chatter), Rule::Full);
}

#[test]
fn problems_name_the_gaps_and_the_duplicates() {
    let problems = table().problems();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("odd") && p.contains("chatter")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("odd") && p.contains("safety") && p.contains("twice")),
        "{problems:?}"
    );
    assert_eq!(problems.len(), 2);
}

#[test]
fn a_game_without_categories_has_no_levels_and_everything_is_full() {
    let none = Verbosity::<NoCategory>::NONE;
    assert!(none.is_empty());
    assert!(none.level("anything").is_none());
    assert!(NoCategory::all().is_empty());
}

#[test]
fn old_settings_without_a_verbosity_read_as_the_first_level() {
    let s: SpeechSettings = serde_json::from_str(r#"{"rate": 0.5}"#).unwrap();
    assert_eq!(s.verbosity, "");
}
