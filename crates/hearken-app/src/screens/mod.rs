//! Screens every game gets.

pub mod bindings;
pub mod confirm;
pub mod help;
#[cfg(feature = "online")]
pub mod online;
pub mod pause;
#[cfg(feature = "online")]
pub mod rooms;
#[cfg(feature = "report")]
pub mod report;
pub mod settings;
pub mod text_entry;
pub mod timed_message;
#[cfg(feature = "update")]
pub mod update;

pub use bindings::{BindingDevice, BindingsScreen};
pub use confirm::{ConfirmOption, ConfirmScreen};
pub use help::HelpScreen;
#[cfg(feature = "online")]
pub use online::OnlineAccountScreen;
pub use pause::PauseScreen;
#[cfg(feature = "online")]
pub use rooms::{MyGamesScreen, OnlinePlayScreen, RoomKind, RoomScreen};
#[cfg(feature = "report")]
pub use report::{ReportPreviewScreen, ReportScreen};
pub use settings::{SettingsRow, SettingsScreen};
pub use text_entry::TextEntryScreen;
pub use timed_message::TimedMessage;
#[cfg(feature = "update")]
pub use update::{UpdateCheckScreen, UpdateDownloadScreen, UpdatePromptScreen, WhatsNewScreen};
