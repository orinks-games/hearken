//! Screens every game gets.

pub mod bindings;
#[cfg(feature = "online")]
pub mod chat;
pub mod confirm;
pub mod help;
#[cfg(feature = "online")]
pub mod lobby;
#[cfg(feature = "online")]
pub mod online;
pub mod pause;
#[cfg(feature = "online")]
pub mod players;
#[cfg(feature = "report")]
pub mod report;
#[cfg(feature = "report")]
pub mod report_link;
#[cfg(feature = "online")]
pub mod rooms;
pub mod settings;
pub mod text_entry;
pub mod timed_message;
#[cfg(feature = "update")]
pub mod update;

pub use bindings::{BindingDevice, BindingsScreen};
#[cfg(feature = "online")]
pub use chat::{ChatListScreen, MutesScreen};
pub use confirm::{ConfirmOption, ConfirmScreen};
pub use help::HelpScreen;
#[cfg(feature = "online")]
pub use lobby::{
    FriendsScreen, InviteScreen, InvitesScreen, LobbyPlayerScreen, NoticeScreen, NoticesScreen,
    PlayOnlineScreen, WhosHereScreen,
};
#[cfg(feature = "online")]
pub use online::OnlineAccountScreen;
pub use pause::PauseScreen;
#[cfg(feature = "online")]
pub use players::PlayersScreen;
#[cfg(feature = "report")]
pub use report::{ReportPreviewScreen, ReportScreen};
#[cfg(feature = "report")]
pub use report_link::ReportLinkScreen;
#[cfg(feature = "online")]
pub use rooms::{enter_room, MyGamesScreen, OnlinePlayScreen, RoomKind, RoomScreen};
pub use settings::{SettingsRow, SettingsScreen};
pub use text_entry::TextEntryScreen;
pub use timed_message::TimedMessage;
#[cfg(feature = "update")]
pub use update::{UpdateCheckScreen, UpdateDownloadScreen, UpdatePromptScreen, WhatsNewScreen};
