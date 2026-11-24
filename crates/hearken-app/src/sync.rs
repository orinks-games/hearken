//! Synced files: a copy of one of the game's data files kept with the
//! player's account, so their progress follows them to another computer.
//!
//! The game owns a [`FileSync`] for each file it syncs and steps it once a
//! frame with [`FileSync::step`]. The sync never reads the file itself: when
//! it needs the file's bytes it answers [`Need::Export`], and when the
//! account's copy has come it answers [`Need::Merge`] with that copy's plain
//! bytes. The game merges in its own way (a higher score kept, unlocks put
//! together) and hands the result back. So nothing here knows what a save
//! holds.
//!
//! Beside the account's copy comes the base: the last of the account's
//! copies this computer's file took in or sent up, kept sealed in the vault
//! as `sync-<name>-base`. With it a game can merge three ways, so a count
//! played up on both computers keeps both sides' plays. It is `None` the
//! first time a computer meets an account.
//!
//! What goes up is sealed with the vault's key ([`Vault::seal_bytes`]), so
//! the server never reads it, and what comes down must open with it
//! ([`Vault::open_bytes`]): a copy is trusted exactly as the file on disk
//! is.
//!
//! Every upload names the account revision it was made from, and the
//! server refuses it when the account has moved on (`409 stale`), so two
//! computers never overwrite each other unseen: the newer copy comes down,
//! is merged, and goes back up.
//!
//! The sync's state is one explicit machine, [`SyncState`], logged on every
//! change under `hearken::sync` ("[sync] up-to-date -> checking"). The
//! first time an account meets a computer that already has progress, the
//! sync stops at [`SyncState::NeedsChoice`] until the game asks the player
//! and calls [`FileSync::choose`].
//!
//! `docs/online.md`, "Synced files", is the protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::online::{Online, OnlineError, Ticket};
use crate::{Opened, Vault};

/// What a game syncs.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncConfig {
    /// The name on the server: 1 to 32 of `a-z`, `0-9` and `-` ("records").
    pub name: String,
    /// The file in the data folder whose copy this is ("records.json"). The
    /// copy is sealed under this name, so it opens only as this file.
    pub file: String,
    /// The version of the file's layout. A copy from a newer version is
    /// left alone: this copy of the game neither merges nor replaces it.
    pub format: u32,
    /// Seconds between one upload and the next, at least.
    pub gap: f64,
    /// Seconds between looks at the account for another computer's
    /// progress, while nothing else is happening.
    pub recheck: f64,
    /// Seconds to wait after the server could not be reached.
    pub retry: f64,
}

impl SyncConfig {
    pub fn new(name: impl Into<String>, file: impl Into<String>, format: u32) -> SyncConfig {
        SyncConfig {
            name: name.into(),
            file: file.into(),
            format,
            gap: 30.0,
            recheck: 600.0,
            retry: 60.0,
        }
    }

    /// The vault file the sync keeps its own notes in.
    fn notes(&self) -> String {
        format!("sync-{}.json", self.name)
    }

    fn base(&self) -> String {
        format!("sync-{}-base", self.name)
    }
}

/// Where a sync is. Every change is logged, and [`SyncState::name`] is
/// what a test or an agent reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// Turned off on this computer by the player.
    Off,
    /// No account is connected, or only a guest.
    NeedsAccount,
    /// About to ask the account for its newest copy.
    Starting,
    /// Asking the account for its newest copy.
    Checking,
    /// Waiting for the game's bytes ([`Need::Export`]).
    Exporting,
    /// Sending this computer's copy.
    Uploading,
    /// Fetching the account's copy.
    Downloading,
    /// Waiting for the game to merge the account's copy ([`Need::Merge`]).
    Merging,
    /// This computer and the account both have progress and have never
    /// met: the player decides ([`FileSync::choose`]).
    NeedsChoice,
    /// The account has everything this computer has.
    UpToDate,
    /// This computer has progress the account has not had yet; it goes up
    /// once the gap since the last upload has passed.
    LocalNewer,
    /// The server could not be reached; tried again later.
    Offline,
    /// Stopped until something changes: see [`Blocked`].
    Blocked(Blocked),
}

/// Why a sync stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// The account's copy is from a newer version of the game.
    NewerFormat,
    /// The file is bigger than the server keeps.
    TooLarge,
    /// The account's copy does not open with this copy's key: a developer
    /// build meeting a release's save, or a damaged copy.
    Unreadable,
    /// The server refused, with its code.
    Refused(String),
}

impl SyncState {
    /// A short stable name: "up-to-date", "needs-choice", "blocked-newer-format".
    pub fn name(&self) -> String {
        match self {
            SyncState::Off => "off".into(),
            SyncState::NeedsAccount => "needs-account".into(),
            SyncState::Starting => "starting".into(),
            SyncState::Checking => "checking".into(),
            SyncState::Exporting => "exporting".into(),
            SyncState::Uploading => "uploading".into(),
            SyncState::Downloading => "downloading".into(),
            SyncState::Merging => "merging".into(),
            SyncState::NeedsChoice => "needs-choice".into(),
            SyncState::UpToDate => "up-to-date".into(),
            SyncState::LocalNewer => "local-newer".into(),
            SyncState::Offline => "offline".into(),
            SyncState::Blocked(Blocked::NewerFormat) => "blocked-newer-format".into(),
            SyncState::Blocked(Blocked::TooLarge) => "blocked-too-large".into(),
            SyncState::Blocked(Blocked::Unreadable) => "blocked-unreadable".into(),
            SyncState::Blocked(Blocked::Refused(code)) => format!("blocked-{code}"),
        }
    }
}

/// What the sync needs from the game before it can go on.
#[derive(Debug, PartialEq, Eq)]
pub enum Need {
    /// The file's plain bytes now, through [`FileSync::exported`].
    Export,
    /// The account's copy, plain, to merge into the game's own, and the
    /// last copy both sides agreed on, when this computer has one; answer
    /// with [`FileSync::merged`].
    Merge {
        theirs: Vec<u8>,
        base: Option<Vec<u8>>,
    },
}

/// Something the player may want to hear about, from
/// [`FileSync::take_events`]. Routine syncing makes none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncEvent {
    /// The account's first copy went up, from this computer.
    FirstSaved,
    /// The server no longer knows the account, so it was disconnected and
    /// syncing stopped (`OnlineError::SignedOut` says what to tell the
    /// player).
    SignedOut,
    /// The account's copy brought something this computer did not have.
    Added,
    /// The sync stopped at [`SyncState::NeedsChoice`].
    NeedsChoice,
    /// The account's copy is from a newer version.
    NewerFormat,
}

/// The player's answer at [`SyncState::NeedsChoice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Merge the two and keep syncing.
    Combine,
    /// Keep them apart: syncing turns off on this computer.
    KeepApart,
}

/// What the sync remembers between runs, sealed in the vault.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Notes {
    #[serde(default = "yes")]
    enabled: bool,
    /// The account this computer last synced with.
    #[serde(default)]
    account: Option<String>,
    /// The account revision this computer's file was last the same as.
    #[serde(default)]
    revision: u64,
    /// The plain file's hash at that revision.
    #[serde(default)]
    hash: String,
    /// The account the kept base belongs to.
    #[serde(default)]
    base: Option<String>,
}

fn yes() -> bool {
    true
}

/// The account's newest copy, as a head answer gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Head {
    account: String,
    save: Option<(u64, u32)>,
}

/// What an export is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    /// Decide what to do with the account's head.
    Decide,
    /// Upload if changed since the last sync.
    UploadIfChanged,
    /// After a merge: upload unless the merge came out the same as the
    /// account's copy.
    AfterMerge,
}

/// One synced file. See the module docs.
pub struct FileSync {
    config: SyncConfig,
    state: SyncState,
    notes: Option<Notes>,
    /// `Online::account_generation` when last looked at.
    generation: Option<u64>,
    ticket: Option<Ticket>,
    head: Option<Head>,
    then: Then,
    /// The account's copy waiting to be merged, its revision and its plain hash.
    cloud: Option<(Vec<u8>, u64, String)>,
    /// The hash and plain bytes being uploaded.
    sending: Option<(String, Vec<u8>)>,
    /// The game said the file may have changed.
    dirty: bool,
    /// When the next upload may go, and the next look at the account.
    next_upload: f64,
    next_check: f64,
    /// `Online::reached` when last looked at: when it grows while offline,
    /// the server answered something else, so it is worth looking again.
    reached: u64,
    /// A first meeting with an account is being settled.
    meeting: bool,
    events: Vec<SyncEvent>,
}

impl FileSync {
    pub fn new(config: SyncConfig) -> FileSync {
        FileSync {
            config,
            state: SyncState::Starting,
            notes: None,
            generation: None,
            ticket: None,
            head: None,
            then: Then::Decide,
            cloud: None,
            sending: None,
            dirty: true,
            next_upload: 0.0,
            next_check: 0.0,
            reached: 0,
            meeting: false,
            events: Vec::new(),
        }
    }

    pub fn config(&self) -> &SyncConfig {
        &self.config
    }

    pub fn state(&self) -> &SyncState {
        &self.state
    }

    /// Whether syncing is on for this computer.
    pub fn enabled(&mut self, vault: &mut Vault) -> bool {
        self.notes(vault).enabled
    }

    /// Turns syncing on or off for this computer. Turning it on looks at
    /// the account at once.
    pub fn set_enabled(&mut self, vault: &mut Vault, on: bool) {
        let mut notes = self.notes(vault).clone();
        notes.enabled = on;
        self.save_notes(vault, notes);
        self.ticket = None;
        self.cloud = None;
        self.go(if on {
            SyncState::Starting
        } else {
            SyncState::Off
        });
    }

    /// The game changed the file: it goes up after the gap.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// Look at the account soon, as on returning to a menu after a while.
    pub fn check_soon(&mut self) {
        if matches!(
            self.state,
            SyncState::UpToDate | SyncState::LocalNewer | SyncState::Offline
        ) {
            self.next_check = 0.0;
        }
    }

    /// What the player should hear about, once.
    pub fn take_events(&mut self) -> Vec<SyncEvent> {
        std::mem::take(&mut self.events)
    }

    /// The player's answer at [`SyncState::NeedsChoice`].
    pub fn choose(&mut self, vault: &mut Vault, choice: Choice) {
        if self.state != SyncState::NeedsChoice {
            return;
        }
        match choice {
            Choice::Combine => self.download(),
            Choice::KeepApart => self.set_enabled(vault, false),
        }
    }

    /// Moves the sync on. `now` is seconds on any steady clock; `safe` is
    /// whether the game could take a merge now (on a menu, not in a run).
    /// Call it once a frame, and again at once after answering a [`Need`].
    pub fn step(
        &mut self,
        online: &mut Online,
        vault: &mut Vault,
        now: f64,
        safe: bool,
    ) -> Option<Need> {
        if let Some(ticket) = self.ticket {
            if self.generation != Some(online.account_generation()) {
                // The account changed under a request: its answer is for the
                // old one. When the answer is what disconnected it, say so.
                if let Some(Err(OnlineError::SignedOut)) = online.result(ticket) {
                    self.events.push(SyncEvent::SignedOut);
                }
                self.ticket = None;
            }
        }
        if self.generation != Some(online.account_generation()) {
            self.generation = Some(online.account_generation());
            self.head = None;
            self.cloud = None;
            if self.state != SyncState::Off {
                self.go(SyncState::Starting);
            }
        }
        if !self.notes(vault).enabled {
            self.go(SyncState::Off);
            return None;
        }
        if !online.is_connected() || online.is_guest() {
            self.go(SyncState::NeedsAccount);
            return None;
        }
        if self.state == SyncState::NeedsAccount || self.state == SyncState::Off {
            self.go(SyncState::Starting);
        }
        if let Some(ticket) = self.ticket {
            let answer = online.result(ticket)?;
            self.ticket = None;
            self.answered(vault, answer, now);
        }
        let back = online.reached() != self.reached;
        self.reached = online.reached();
        match self.state.clone() {
            SyncState::Starting => self.send_head(online, now),
            SyncState::Offline if back || now >= self.next_check => self.send_head(online, now),
            SyncState::UpToDate | SyncState::LocalNewer if now >= self.next_check => {
                self.send_head(online, now)
            }
            SyncState::UpToDate if self.dirty && now >= self.next_upload => {
                self.then = Then::UploadIfChanged;
                self.go(SyncState::Exporting);
                return Some(Need::Export);
            }
            SyncState::LocalNewer if now >= self.next_upload => {
                self.then = Then::UploadIfChanged;
                self.go(SyncState::Exporting);
                return Some(Need::Export);
            }
            SyncState::Exporting => return Some(Need::Export),
            SyncState::Downloading if self.ticket.is_none() => {
                match online.save_download(&self.config.name) {
                    Ok(t) => self.ticket = Some(t),
                    Err(e) => self.failed(&e, now),
                }
            }
            SyncState::Merging if safe => {
                if let Some((plain, _, _)) = &self.cloud {
                    let theirs = plain.clone();
                    let base = self.read_base(vault);
                    return Some(Need::Merge { theirs, base });
                }
                self.go(SyncState::Starting);
            }
            SyncState::Uploading if self.ticket.is_none() => self.go(SyncState::Starting),
            _ => {}
        }
        None
    }

    /// The game's answer to [`Need::Export`]: the file's plain bytes,
    /// whether it holds any progress worth keeping (false for a fresh
    /// file), and the card to send beside it.
    pub fn exported(
        &mut self,
        online: &mut Online,
        vault: &mut Vault,
        plain: &[u8],
        has_progress: bool,
        card: Option<Value>,
        now: f64,
    ) {
        if self.state != SyncState::Exporting {
            return;
        }
        // The export holds everything the game has changed so far.
        self.dirty = false;
        let hash = sha256_hex(plain);
        let notes = self.notes(vault).clone();
        let head = self.head.clone();
        let account = head.as_ref().map(|h| h.account.clone());
        match self.then {
            Then::Decide => {
                let Some(head) = head else {
                    self.go(SyncState::Starting);
                    return;
                };
                let met = notes.account.as_deref() == Some(head.account.as_str());
                match head.save {
                    None if !has_progress => {
                        // Nothing on either side yet: nothing to send until
                        // there is, so the account's first copy holds
                        // progress.
                        let mut notes = notes;
                        notes.account = Some(head.account);
                        notes.revision = 0;
                        notes.hash = String::new();
                        self.save_notes(vault, notes);
                        self.dirty = false;
                        self.go(SyncState::UpToDate);
                    }
                    None => {
                        self.meeting = !met;
                        self.upload(online, vault, plain, hash, 0, card, now);
                    }
                    Some(_) if !met && has_progress => {
                        self.meeting = true;
                        self.events.push(SyncEvent::NeedsChoice);
                        self.go(SyncState::NeedsChoice);
                    }
                    Some(_) if !met => {
                        self.meeting = true;
                        self.download();
                    }
                    Some((revision, _)) if revision == notes.revision => {
                        if hash == notes.hash {
                            self.dirty = false;
                            self.go(SyncState::UpToDate);
                        } else {
                            self.upload(online, vault, plain, hash, revision, card, now);
                        }
                    }
                    Some(_) => self.download(),
                }
            }
            Then::UploadIfChanged => {
                self.dirty = false;
                let account_empty = head.as_ref().is_some_and(|h| h.save.is_none());
                if (hash == notes.hash && account == notes.account)
                    || (account_empty && !has_progress)
                {
                    self.go(SyncState::UpToDate);
                } else {
                    let base = notes.revision;
                    self.upload(online, vault, plain, hash, base, card, now);
                }
            }
            Then::AfterMerge => {
                let Some((_, revision, cloud_hash)) = self.cloud.take() else {
                    self.go(SyncState::Starting);
                    return;
                };
                let meeting = std::mem::take(&mut self.meeting);
                if hash == cloud_hash {
                    self.save_base(vault, plain);
                    let mut notes = notes;
                    notes.account = account;
                    notes.revision = revision;
                    notes.hash = hash;
                    self.save_notes(vault, notes);
                    self.dirty = false;
                    self.go(SyncState::UpToDate);
                } else {
                    self.meeting = meeting;
                    self.upload(online, vault, plain, hash, revision, card, now);
                }
            }
        }
    }

    /// The game's answer to [`Need::Merge`]: whether the merge changed this
    /// computer's file. The sync then asks for an export, to send the
    /// merged file up unless it came out the same as the account's.
    pub fn merged(&mut self, vault: &mut Vault, changed: bool) {
        if self.state != SyncState::Merging {
            return;
        }
        // This computer's file now holds the account's copy, so that copy is
        // where the next merge starts from, even if the upload never goes.
        if let Some(plain) = self.cloud.as_ref().map(|(plain, _, _)| plain.clone()) {
            self.save_base(vault, &plain);
        }
        if changed {
            self.events.push(SyncEvent::Added);
        }
        self.then = Then::AfterMerge;
        self.go(SyncState::Exporting);
    }

    /// Sends this computer's file up now and waits up to `timeout` for the
    /// answer, as a game does as it closes. Only when the file changed
    /// since the last sync and the sync is otherwise settled.
    pub fn flush(
        &mut self,
        online: &mut Online,
        vault: &mut Vault,
        plain: &[u8],
        has_progress: bool,
        card: Option<Value>,
        timeout: std::time::Duration,
    ) {
        if !matches!(self.state, SyncState::UpToDate | SyncState::LocalNewer) {
            return;
        }
        let notes = self.notes(vault).clone();
        let hash = sha256_hex(plain);
        if hash == notes.hash || notes.account.is_none() {
            return;
        }
        self.go(SyncState::Exporting);
        self.then = Then::UploadIfChanged;
        self.exported(online, vault, plain, has_progress, card, f64::MAX);
        if let Some(ticket) = self.ticket.take() {
            if let Some(answer) = online.wait(ticket, timeout) {
                self.answered(vault, answer, f64::MAX);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn upload(
        &mut self,
        online: &mut Online,
        vault: &mut Vault,
        plain: &[u8],
        hash: String,
        base: u64,
        card: Option<Value>,
        now: f64,
    ) {
        let sealed = match vault.seal_bytes(&self.config.file, plain) {
            Ok(sealed) => sealed,
            Err(e) => {
                log::warn!("[sync] could not seal {}: {e}", self.config.file);
                self.go(SyncState::Blocked(Blocked::Refused("seal".into())));
                return;
            }
        };
        match online.save_upload(&self.config.name, base, self.config.format, &sealed, card) {
            Ok(ticket) => {
                self.ticket = Some(ticket);
                self.sending = Some((hash, plain.to_vec()));
                self.next_upload = now + self.config.gap;
                self.go(SyncState::Uploading);
            }
            Err(e) => self.failed(&e, now),
        }
    }

    fn download(&mut self) {
        self.ticket = None;
        self.go(SyncState::Downloading);
    }

    fn send_head(&mut self, online: &mut Online, now: f64) {
        match online.save_head(&self.config.name) {
            Ok(ticket) => {
                self.ticket = Some(ticket);
                self.go(SyncState::Checking);
            }
            // Other requests fill the queue: the next frame tries again.
            Err(OnlineError::Busy) => {}
            Err(e) => self.failed(&e, now),
        }
    }

    fn answered(&mut self, vault: &mut Vault, answer: Result<Value, OnlineError>, now: f64) {
        let body = match answer {
            Ok(body) => body,
            Err(OnlineError::Refused { code, .. }) if code == "stale" => {
                // The account moved on: look again, merge, try again.
                self.go(SyncState::Starting);
                return;
            }
            Err(e) => {
                self.failed(&e, now);
                return;
            }
        };
        match self.state {
            SyncState::Checking => {
                let account = body["account"].as_str().unwrap_or("").to_string();
                if account.is_empty() {
                    self.failed(&OnlineError::Unavailable, now);
                    return;
                }
                let save = match &body["save"] {
                    Value::Null => None,
                    save => match (save["revision"].as_u64(), save["format"].as_u64()) {
                        (Some(r), Some(f)) => Some((r, f as u32)),
                        _ => {
                            self.failed(&OnlineError::Unavailable, now);
                            return;
                        }
                    },
                };
                if save.is_some_and(|(_, format)| format > self.config.format) {
                    self.events.push(SyncEvent::NewerFormat);
                    self.go(SyncState::Blocked(Blocked::NewerFormat));
                    return;
                }
                self.head = Some(Head { account, save });
                self.next_check = now + self.config.recheck;
                self.then = Then::Decide;
                self.go(SyncState::Exporting);
            }
            SyncState::Downloading => {
                let (Some(revision), Some(content), Some(hash)) = (
                    body["revision"].as_u64(),
                    body["content"].as_str().and_then(unbase64),
                    body["hash"].as_str(),
                ) else {
                    self.failed(&OnlineError::Unavailable, now);
                    return;
                };
                if sha256_hex(&content) != hash {
                    self.failed(&OnlineError::Unavailable, now);
                    return;
                }
                if body["format"]
                    .as_u64()
                    .is_some_and(|f| f > self.config.format as u64)
                {
                    self.events.push(SyncEvent::NewerFormat);
                    self.go(SyncState::Blocked(Blocked::NewerFormat));
                    return;
                }
                match vault.open_bytes(&self.config.file, &content) {
                    Opened::Sealed(plain) | Opened::Plain(plain) => {
                        let plain_hash = sha256_hex(&plain);
                        self.cloud = Some((plain, revision, plain_hash));
                        self.go(SyncState::Merging);
                    }
                    other => {
                        log::warn!(
                            "[sync] the account's {} does not open here: {other:?}",
                            self.config.file
                        );
                        self.go(SyncState::Blocked(Blocked::Unreadable));
                    }
                }
            }
            SyncState::Uploading => {
                let (Some(revision), Some((hash, plain))) =
                    (body["revision"].as_u64(), self.sending.take())
                else {
                    self.failed(&OnlineError::Unavailable, now);
                    return;
                };
                let account = self.head.as_ref().map(|h| h.account.clone());
                let mut notes = self.notes(vault).clone();
                notes.account = account.or(notes.account);
                notes.revision = revision;
                notes.hash = hash;
                self.save_notes(vault, notes);
                self.save_base(vault, &plain);
                if let Some(head) = &mut self.head {
                    head.save = Some((revision, self.config.format));
                }
                self.meeting = false;
                if revision == 1 {
                    self.events.push(SyncEvent::FirstSaved);
                }
                self.go(if self.dirty {
                    SyncState::LocalNewer
                } else {
                    SyncState::UpToDate
                });
            }
            _ => {}
        }
    }

    fn failed(&mut self, error: &OnlineError, now: f64) {
        log::info!("[sync] {}: {}", self.config.name, error.code());
        self.ticket = None;
        self.sending = None;
        let state = match error {
            OnlineError::NotConnected => SyncState::NeedsAccount,
            OnlineError::SignedOut => {
                self.events.push(SyncEvent::SignedOut);
                SyncState::NeedsAccount
            }
            OnlineError::Refused { code, .. } if code == "account-needed" => {
                SyncState::NeedsAccount
            }
            OnlineError::Refused { code, .. } if code == "too-large" => {
                SyncState::Blocked(Blocked::TooLarge)
            }
            OnlineError::Refused { code, .. } => SyncState::Blocked(Blocked::Refused(code.clone())),
            _ => {
                self.next_check = now + self.config.retry;
                SyncState::Offline
            }
        };
        self.go(state);
    }

    fn go(&mut self, state: SyncState) {
        if self.state != state {
            log::info!(target: "hearken::sync", "[sync] {} -> {}", self.state.name(), state.name());
            self.state = state;
        }
    }

    fn notes(&mut self, vault: &mut Vault) -> &Notes {
        if self.notes.is_none() {
            let name = self.config.notes();
            let notes = match vault.read(&name) {
                Opened::Sealed(bytes) | Opened::Plain(bytes) => serde_json::from_slice(&bytes)
                    .unwrap_or_else(|_| Notes {
                        enabled: true,
                        ..Notes::default()
                    }),
                _ => Notes {
                    enabled: true,
                    ..Notes::default()
                },
            };
            self.notes = Some(notes);
        }
        self.notes.as_ref().expect("just read")
    }

    /// The last copy both sides agreed on: only when it is the one the
    /// notes name, for the account this computer last synced with.
    fn read_base(&mut self, vault: &mut Vault) -> Option<Vec<u8>> {
        let account = self.head.as_ref().map(|h| h.account.clone());
        if account.is_none() || self.notes(vault).base != account {
            return None;
        }
        match vault.read(&self.config.base()) {
            Opened::Sealed(bytes) | Opened::Plain(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Keeps `plain` as the base: a copy the account holds and this
    /// computer's file now includes.
    fn save_base(&mut self, vault: &mut Vault, plain: &[u8]) {
        let name = self.config.base();
        if let Err(e) = vault.write(&name, plain) {
            log::warn!("[sync] could not save {name}: {e}");
            return;
        }
        let mut notes = self.notes(vault).clone();
        notes.base = self.head.as_ref().map(|h| h.account.clone());
        self.save_notes(vault, notes);
    }

    fn save_notes(&mut self, vault: &mut Vault, notes: Notes) {
        let name = self.config.notes();
        match serde_json::to_vec(&notes) {
            Ok(bytes) => {
                if let Err(e) = vault.write(&name, &bytes) {
                    log::warn!("[sync] could not save {name}: {e}");
                }
            }
            Err(e) => log::warn!("[sync] could not write {name}: {e}"),
        }
        self.notes = Some(notes);
    }
}

/// Lowercase hex SHA-256, as the server checks.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64, padded.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes of standard padded base64, or `None` when it is not.
pub fn unbase64(text: &str) -> Option<Vec<u8>> {
    let text = text.as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let value = |c: u8| ALPHABET.iter().position(|&a| a == c).map(|v| v as u32);
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for (i, chunk) in text.chunks(4).enumerate() {
        let last = i == text.len() / 4 - 1;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for &c in &chunk[..4 - pad] {
            n = n << 6 | value(c)?;
        }
        n <<= 6 * pad as u32;
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;
