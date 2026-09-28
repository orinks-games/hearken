//! A generic verbosity table a game fills with its own categories and levels.
//!
//! A game names its event categories (safety alerts, chatter) and verbosity
//! levels (full, quiet, silent) -- what each category does at each level. The
//! engine applies those rules to every spoken event: when the player asks for
//! quiet mode, events marked "chatter" become earcons instead of speech.

/// A category of spoken events a game defines.
///
/// Each category has a static list returned by `all()` and a string id
/// for diagnostics.
pub trait Category: Copy + Eq + std::hash::Hash + std::fmt::Debug + 'static {
    /// All categories this game defines.
    fn all() -> &'static [Self];
    /// The diagnostic string for this category.
    fn id(self) -> &'static str;
}

/// A marker category for games with no event categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoCategory {}

impl Category for NoCategory {
    fn all() -> &'static [Self] {
        &[]
    }

    fn id(self) -> &'static str {
        match self {}
    }
}

/// What to do with an event at a verbosity level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Speak the full text.
    Full,
    /// Speak a brief version.
    Terse,
    /// Speak only the first time, then remain silent.
    FirstTime,
    /// Play a sound effect instead of speaking.
    Earcon(&'static str),
    /// Speak nothing.
    Silent,
}

/// A verbosity level that defines what each category does.
#[derive(Clone, Copy, Debug)]
pub struct Level<C: 'static> {
    /// The level's id: a key a game uses to request this level.
    pub id: &'static str,
    /// Human-readable label for menus.
    pub label: &'static str,
    /// Rules mapping each category to its behavior, in order. The first rule
    /// for a category is used; duplicates are a data error.
    pub rules: &'static [(C, Rule)],
}

/// A verbosity table: the levels a game defines.
#[derive(Debug)]
pub struct Verbosity<C: 'static> {
    /// The levels in order; empty means no levels defined.
    pub levels: &'static [Level<C>],
}

// Implement Clone and Copy by hand without requiring C: Clone or C: Copy.
// Copy is a marker trait; it requires Clone, which requires C to be cloneable.
// But Verbosity only holds a &'static slice, so it is always cloneable.
impl<C: 'static> Clone for Verbosity<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: 'static> Copy for Verbosity<C> {}

impl<C: Category> Verbosity<C> {
    /// An empty verbosity table.
    pub const NONE: Verbosity<C> = Verbosity { levels: &[] };

    /// Whether this table has no levels.
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// Look up a level by id. Returns that level if found, else the first level
    /// if any exist, else `None`.
    pub fn level(&self, id: &str) -> Option<&'static Level<C>> {
        if let Some(level) = self.levels.iter().find(|l| l.id == id) {
            return Some(level);
        }
        self.levels.first()
    }

    /// The rule for a category at a given level. Returns the first matching
    /// rule from that level, or `Rule::Full` if the category is unlisted or no
    /// levels exist.
    pub fn rule(&self, level_id: &str, category: C) -> Rule {
        match self.level(level_id) {
            Some(level) => level
                .rules
                .iter()
                .find(|(cat, _)| *cat == category)
                .map(|(_, rule)| *rule)
                .unwrap_or(Rule::Full),
            None => Rule::Full,
        }
    }

    /// Configuration problems: missing categories, duplicate level ids, and
    /// duplicate category entries within a level.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();

        // Check for duplicate level ids
        for (i, level) in self.levels.iter().enumerate() {
            if self.levels[..i].iter().any(|l| l.id == level.id) {
                problems.push(format!("level {} appears twice", level.id));
            }
        }

        // For each level, check for missing categories and duplicates
        for level in self.levels {
            let mut seen = std::collections::HashSet::new();
            for (cat, _) in level.rules {
                if !seen.insert(cat.id()) {
                    problems.push(format!("level {} lists {} twice", level.id, cat.id()));
                }
            }
            for cat in C::all() {
                if !level.rules.iter().any(|(c, _)| *c == *cat) {
                    problems.push(format!("level {} has no rule for {}", level.id, cat.id()));
                }
            }
        }

        problems
    }
}
