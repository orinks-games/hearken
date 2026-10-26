//! The verbosity rules, applied to a line before any voice says it.

use std::collections::HashSet;

use hearken_app::{Bus, Ctx, Game, Line, Rule, TRANSCRIPT_TARGET};

/// What `line` says at the player's verbosity level, or `None` when it says
/// nothing (silenced, an earcon instead, a first-time line already heard).
pub(crate) fn resolve<G: Game>(
    ctx: &mut Ctx<G>,
    line: &Line<G::Category>,
    first_times: &mut HashSet<(String, String)>,
) -> Option<String> {
    let rule = match line.category {
        Some(c) => ctx.verbosity().rule(&ctx.settings.speech.verbosity, c),
        None => Rule::Full,
    };
    let text = match rule {
        Rule::Silent | Rule::Earcon(_) => {
            if let Rule::Earcon(sound) = rule {
                ctx.audio.play(sound, Bus::UI);
            }
            log::info!(target: TRANSCRIPT_TARGET, "[verbosity] not voiced: {}", line.text);
            return None;
        }
        Rule::FirstTime => {
            let seen = (line.key.clone().unwrap_or_default(), line.text.clone());
            if !first_times.insert(seen) {
                return None;
            }
            line.text.clone()
        }
        Rule::Terse => line.terse.clone().unwrap_or_else(|| line.text.clone()),
        Rule::Full => line.text.clone(),
    };
    (!text.trim().is_empty()).then_some(text)
}
