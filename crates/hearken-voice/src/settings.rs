//! The player's voice settings, and the Settings rows that change them.

use std::collections::BTreeMap;

use hearken_app::menu::{Label, Menu, MenuItem};
use hearken_app::screens::SettingsScreen;
use hearken_app::{Ctx, EventPriority, Game, Line, SettingsRow};
use serde::{Deserialize, Serialize};

/// Who speaks commentary.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VoiceMode {
    /// The AI voices, or the screen reader while they are not available.
    #[default]
    Ai,
    /// The screen reader's event voice, through the verbosity rules.
    ScreenReader,
    /// No commentary at all.
    Off,
}

impl VoiceMode {
    const ALL: [VoiceMode; 3] = [VoiceMode::Ai, VoiceMode::ScreenReader, VoiceMode::Off];

    pub fn label(self) -> &'static str {
        match self {
            VoiceMode::Ai => "AI voices",
            VoiceMode::ScreenReader => "screen reader",
            VoiceMode::Off => "off",
        }
    }

    fn step(self, delta: i32) -> VoiceMode {
        let i = VoiceMode::ALL.iter().position(|m| *m == self).unwrap_or(0) as i32;
        VoiceMode::ALL[(i + delta).rem_euclid(3) as usize]
    }
}

/// One of the game's speakers, such as the play-by-play announcer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role {
    /// What a `Line`'s `speaker` names.
    pub id: &'static str,
    /// What Settings calls the role: "Play-by-play".
    pub label: &'static str,
    /// The voice until the player picks another: `"bf_emma"`.
    pub default_voice: &'static str,
    /// What Enter on the role's Settings row plays.
    pub sample: &'static str,
}

/// Saved with the game's own settings (`G::Settings`, which implements
/// [`HasVoiceSettings`]).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct VoiceSettings {
    pub mode: VoiceMode,
    /// The voice for each role id; a role with none uses its default.
    pub speakers: BTreeMap<String, String>,
    /// 1 is normal; 0.5 to 2.
    pub rate: f32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        VoiceSettings {
            mode: VoiceMode::Ai,
            speakers: BTreeMap::new(),
            rate: 1.0,
        }
    }
}

impl VoiceSettings {
    /// The voice `role` speaks in: the player's pick, the role's default,
    /// or, for a role the game did not list, the role id itself.
    pub fn voice_for(&self, role: &str, roles: &[Role]) -> String {
        if let Some(voice) = self.speakers.get(role) {
            return voice.clone();
        }
        roles
            .iter()
            .find(|r| r.id == role)
            .map(|r| r.default_voice)
            .unwrap_or(role)
            .to_string()
    }

    /// The rate, clamped, with a damaged value read as normal.
    pub fn rate(&self) -> f32 {
        if self.rate.is_finite() {
            self.rate.clamp(0.5, 2.0)
        } else {
            1.0
        }
    }
}

/// Implemented by a game's `Settings` type to say where its
/// [`VoiceSettings`] are kept.
pub trait HasVoiceSettings {
    fn voice_settings(&self) -> &VoiceSettings;
    fn voice_settings_mut(&mut self) -> &mut VoiceSettings;
}

fn voice<G: Game>(ctx: &Ctx<G>) -> &VoiceSettings
where
    G::Settings: HasVoiceSettings,
{
    ctx.settings.game.voice_settings()
}

fn voice_mut<G: Game>(ctx: &mut Ctx<G>) -> &mut VoiceSettings
where
    G::Settings: HasVoiceSettings,
{
    ctx.settings.game.voice_settings_mut()
}

fn mode_label<G: Game>(ctx: &Ctx<G>) -> String
where
    G::Settings: HasVoiceSettings,
{
    let mode = voice(ctx).mode;
    let unavailable = mode == VoiceMode::Ai && ctx.commentary_status().is_some();
    if unavailable {
        "Commentary voice: AI voices, not available, using the screen reader".to_string()
    } else {
        format!("Commentary voice: {}", mode.label())
    }
}

fn mode_help<G: Game>(ctx: &Ctx<G>) -> String {
    let mut help = "Who speaks the commentary: the AI voices, your screen reader, or nobody. \
                    Left and Right change it."
        .to_string();
    if let Some(status) = ctx.commentary_status() {
        help.push(' ');
        help.push_str(&status);
    }
    help
}

fn step_mode<G: Game>(ctx: &mut Ctx<G>, delta: i32)
where
    G::Settings: HasVoiceSettings,
{
    let next = voice(ctx).mode.step(delta);
    voice_mut(ctx).mode = next;
    if next != VoiceMode::Ai {
        ctx.stop_commentary();
    }
}

/// The voices a role can take: the loaded model's, else Kokoro's list.
fn choices<G: Game>(ctx: &Ctx<G>) -> Vec<String> {
    let loaded = ctx.commentary_speakers();
    if loaded.is_empty() {
        crate::KOKORO_VOICES.iter().map(|v| v.to_string()).collect()
    } else {
        loaded
    }
}

fn step_speaker<G: Game>(ctx: &mut Ctx<G>, role: &Role, roles: &'static [Role], delta: i32)
where
    G::Settings: HasVoiceSettings,
{
    let choices = choices(ctx);
    if choices.is_empty() {
        return;
    }
    let current = voice(ctx).voice_for(role.id, roles);
    let i = choices.iter().position(|v| *v == current).unwrap_or(0) as i32;
    let next = choices[(i + delta).rem_euclid(choices.len() as i32) as usize].clone();
    voice_mut(ctx).speakers.insert(role.id.to_string(), next);
}

fn rate_label(rate: f32) -> String {
    format!(
        "Commentary speed: {} percent",
        (rate * 100.0).round() as i64
    )
}

/// The Settings rows for the voices, for a game's `Game::settings_rows` to
/// add: who speaks commentary, a voice for each of `roles` (Enter plays its
/// sample), and the speed.
pub fn settings_rows<G: Game>(roles: &'static [Role]) -> Vec<SettingsRow<G>>
where
    G::Settings: HasVoiceSettings,
{
    type S<G> = SettingsScreen<G>;
    let mut rows = vec![SettingsRow::new(
        "voice_mode",
        MenuItem::new(
            Label::dynamic(|_s: &S<G>, ctx: &Ctx<G>| mode_label(ctx)),
            |s: &mut S<G>, ctx: &mut Ctx<G>| {
                step_mode(ctx, 1);
                s.refresh(ctx, true);
                s.speak_current(ctx);
            },
        )
        .help(Label::dynamic(|_s: &S<G>, ctx: &Ctx<G>| mode_help(ctx)))
        .adjustable(|_s: &mut S<G>, ctx: &mut Ctx<G>, delta| step_mode(ctx, delta)),
    )];
    for role in roles {
        rows.push(SettingsRow::new(
            format!("voice_speaker_{}", role.id),
            MenuItem::new(
                Label::dynamic(move |_s: &S<G>, ctx: &Ctx<G>| {
                    format!(
                        "{} voice: {}",
                        role.label,
                        voice(ctx).voice_for(role.id, roles)
                    )
                }),
                move |_s: &mut S<G>, ctx: &mut Ctx<G>| {
                    ctx.stop_commentary();
                    ctx.commentate(
                        Line::new(role.id, role.sample).priority(EventPriority::Critical),
                    );
                },
            )
            .help(format!(
                "The voice {} speaks in. Left and Right change it, Enter plays a sample.",
                role.label
            ))
            .adjustable(move |_s: &mut S<G>, ctx: &mut Ctx<G>, delta| {
                step_speaker(ctx, role, roles, delta)
            }),
        ));
    }
    rows.push(SettingsRow::new(
        "voice_rate",
        MenuItem::inert(Label::dynamic(|_s: &S<G>, ctx: &Ctx<G>| {
            rate_label(voice(ctx).rate())
        }))
        .help("How fast the AI voices speak. Left and Right change it.")
        .adjustable(|_s: &mut S<G>, ctx: &mut Ctx<G>, delta| {
            let next = voice(ctx).rate() + delta as f32 * 0.1;
            voice_mut(ctx).rate = ((next * 10.0).round() / 10.0).clamp(0.5, 2.0);
        }),
    ));
    #[cfg(feature = "download")]
    rows.push(SettingsRow::new(
        "voice_install",
        MenuItem::new(
            Label::dynamic(|_s: &S<G>, ctx: &Ctx<G>| {
                if ctx.commentary_status().as_deref() == Some(crate::NOT_INSTALLED) {
                    "Download the AI voices".to_string()
                } else {
                    "Download the AI voices again".to_string()
                }
            }),
            |_s: &mut S<G>, ctx: &mut Ctx<G>| {
                let voices = ctx.paths.data_dir.join("voices");
                ctx.push(crate::install_confirm::<G>(voices));
            },
        )
        .help("Downloads the voices the AI commentary speaks with, about 92 megabytes."),
    ));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLES: &[Role] = &[Role {
        id: "pbp",
        label: "Play-by-play",
        default_voice: "bf_emma",
        sample: "Welcome in.",
    }];

    #[test]
    fn a_role_speaks_in_its_default_until_the_player_picks() {
        let mut settings = VoiceSettings::default();
        assert_eq!(settings.voice_for("pbp", ROLES), "bf_emma");
        assert_eq!(settings.voice_for("am_onyx", ROLES), "am_onyx");
        settings.speakers.insert("pbp".into(), "af_nova".into());
        assert_eq!(settings.voice_for("pbp", ROLES), "af_nova");
    }

    #[test]
    fn modes_cycle_both_ways() {
        assert_eq!(VoiceMode::Ai.step(1), VoiceMode::ScreenReader);
        assert_eq!(VoiceMode::Ai.step(-1), VoiceMode::Off);
    }

    #[test]
    fn old_files_read_with_defaults() {
        let s: VoiceSettings = serde_json::from_str(r#"{"mode":"off"}"#).unwrap();
        assert_eq!(s.mode, VoiceMode::Off);
        assert_eq!(s.rate, 1.0);
    }
}
