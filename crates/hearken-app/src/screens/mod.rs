//! Screens every game gets.

pub mod bindings;
pub mod confirm;
pub mod help;
#[cfg(feature = "online")]
pub mod online;
pub mod pause;
pub mod settings;
pub mod text_entry;
pub mod timed_message;

pub use bindings::{BindingDevice, BindingsScreen};
pub use confirm::{ConfirmOption, ConfirmScreen};
pub use help::HelpScreen;
#[cfg(feature = "online")]
pub use online::OnlineAccountScreen;
pub use pause::PauseScreen;
pub use settings::{SettingsRow, SettingsScreen};
pub use text_entry::TextEntryScreen;
pub use timed_message::TimedMessage;
