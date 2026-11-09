//! Report a problem: what happened, an address for a reply, whether the
//! log goes along, a read-through of the report, and Send.
//!
//! Sending runs on a worker thread; the screen polls it each frame and
//! speaks the outcome, so the game never waits on the network and Escape
//! always leaves at once. A report the relay will not take can be saved to
//! a file in the data folder instead.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use hearken_report::{Invalid, Report, SendError, MAX_DESCRIPTION_CHARS};

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::InputEvent;
use crate::menu::{menu_input, Label, Menu, MenuCore, MenuItem};
use crate::reports::Sending;
use crate::screens::{ConfirmOption, ConfirmScreen, TextEntryScreen};
use crate::sound::UiSound;
use crate::text::end_sentence;

/// Spoken when Send is chosen with nothing written.
pub const DESCRIBE_FIRST: &str = "Describe what happened first.";
/// Spoken once the relay took the report.
pub const SENT: &str = "Thank you. Your report was sent.";
/// Spoken when the reply address cannot receive mail.
pub const BAD_ADDRESS: &str =
    "Email cannot be delivered to that address. Check it, or leave it blank.";

/// How the What happened field works, spoken on arrival and on F1.
const DESCRIPTION_INSTRUCTIONS: &str = "Say what you were doing and what went wrong. Type, then \
     press Enter. Left and Right review what you have typed, Home and End jump to the start or \
     end, and Up reads it all. Escape keeps what was there.";

/// The answers so far, shared with the fields that fill them in.
#[derive(Default)]
struct Draft {
    description: String,
    reply_to: String,
    include_logs: bool,
    /// The file the report was saved to, once it was.
    saved: Option<PathBuf>,
}

pub struct ReportScreen<G: Game> {
    menu: MenuCore<Self, G>,
    draft: Rc<RefCell<Draft>>,
    /// Whether this report follows a run that did not exit cleanly.
    crashed: bool,
    sending: Option<Sending>,
    /// The report in flight, kept to save if the relay will not take it.
    report: Option<Report>,
    /// A row to move to and read on the next frame: the rows are lent out
    /// while an action runs, so the move waits until they are back.
    focus_next: Option<usize>,
}

impl<G: Game> ReportScreen<G> {
    /// A report of a problem the player noticed.
    pub fn new() -> Self {
        ReportScreen::with_crashed(false)
    }

    /// A report of the last run closing unexpectedly, offered at start.
    pub fn after_crash() -> Self {
        ReportScreen::with_crashed(true)
    }

    fn with_crashed(crashed: bool) -> Self {
        ReportScreen {
            menu: MenuCore::new("Report a problem")
                .with_intro_help("Up and Down move, Enter chooses, Escape goes back."),
            draft: Rc::new(RefCell::new(Draft {
                include_logs: true,
                ..Draft::default()
            })),
            crashed,
            sending: None,
            report: None,
            focus_next: None,
        }
    }

    /// The row for What happened or the reply address, which opens a text
    /// field over this screen.
    fn text_row(&self, label: &'static str, field: Field) -> MenuItem<Self, G> {
        let draft = self.draft.clone();
        MenuItem::new(
            Label::dynamic(move |_s: &Self, _ctx: &Ctx<G>| {
                let d = draft.borrow();
                let value = match field {
                    Field::Description if d.description.is_empty() => "nothing written yet",
                    Field::Description => d.description.as_str(),
                    Field::ReplyTo if d.reply_to.is_empty() => "none",
                    Field::ReplyTo => d.reply_to.as_str(),
                };
                format!("{label}: {value}")
            }),
            move |s: &mut Self, ctx: &mut Ctx<G>| s.open_field(ctx, field),
        )
        .help(match field {
            Field::Description => {
                "What you were doing, and what went wrong. Enter opens the field."
            }
            Field::ReplyTo => {
                "Where a reply can reach you. Leave it blank to send the report without one."
            }
        })
    }

    fn open_field(&mut self, ctx: &mut Ctx<G>, field: Field) {
        let draft = self.draft.clone();
        let current = {
            let d = draft.borrow();
            match field {
                Field::Description => d.description.clone(),
                Field::ReplyTo => d.reply_to.clone(),
            }
        };
        let screen = match field {
            Field::Description => TextEntryScreen::<G>::new("What happened", "", move |ctx, text| {
                draft.borrow_mut().description = text.trim().to_string();
                ctx.pop();
            })
            .instructions(DESCRIPTION_INSTRUCTIONS)
            .max_len(MAX_DESCRIPTION_CHARS),
            Field::ReplyTo => TextEntryScreen::<G>::new(
                "Your email address, for a reply",
                "",
                move |ctx, text| {
                    let text = text.trim();
                    if !text.is_empty() && !looks_like_address(text) {
                        ctx.play_ui(UiSound::Error);
                        ctx.say("That does not look like an email address. Check it, or leave it blank.");
                        return;
                    }
                    draft.borrow_mut().reply_to = text.to_string();
                    ctx.pop();
                },
            )
            .max_len(254),
        };
        ctx.push(screen.with_text(&current));
    }

    fn toggle_logs(&mut self, ctx: &mut Ctx<G>) {
        {
            let mut d = self.draft.borrow_mut();
            d.include_logs = !d.include_logs;
        }
        self.refresh(ctx, true);
        self.speak_current(ctx);
    }

    /// The report as the answers stand, or `None` with the reason spoken.
    fn report(&self, ctx: &mut Ctx<G>) -> Option<Report> {
        let d = self.draft.borrow();
        if d.description.trim().is_empty() {
            drop(d);
            ctx.play_ui(UiSound::Error);
            ctx.say(DESCRIBE_FIRST);
            return None;
        }
        ctx.reports
            .draft(&d.description, &d.reply_to, d.include_logs, self.crashed)
    }

    fn read_report(&mut self, ctx: &mut Ctx<G>) {
        if let Some(report) = self.report(ctx) {
            ctx.push(ReportPreviewScreen::<G>::new(&report));
        }
    }

    fn send(&mut self, ctx: &mut Ctx<G>) {
        if self.sending.is_some() {
            ctx.say("Still sending the report.");
            return;
        }
        let Some(report) = self.report(ctx) else {
            self.focus_next = Some(0);
            return;
        };
        if let Err(invalid) = report.validate() {
            ctx.play_ui(UiSound::Error);
            ctx.say(invalid_line(&invalid));
            return;
        }
        self.sending = Some(ctx.reports.send(report.clone()));
        self.report = Some(report);
        ctx.say("Sending the report.");
    }

    /// The relay would not take the report, or could not be reached: offer
    /// to save it to a file instead.
    fn offer_to_save(&mut self, ctx: &mut Ctx<G>, problem: String) {
        let Some(report) = self.report.take() else {
            return;
        };
        let draft = self.draft.clone();
        ctx.play_ui(UiSound::Error);
        ctx.push(
            ConfirmScreen::with_options(
                format!(
                    "{} Save the report to a file instead, to send another way?",
                    end_sentence(&problem)
                ),
                vec![
                    ConfirmOption::new("Save the report", move |ctx: &mut Ctx<G>| {
                        match ctx.reports.save(&report, &ctx.paths) {
                            Ok(path) => draft.borrow_mut().saved = Some(path),
                            Err(e) => {
                                ctx.play_ui(UiSound::Error);
                                ctx.say(format!("The report could not be saved: {e}."));
                            }
                        }
                    }),
                    ConfirmOption::close("Not now"),
                ],
            )
            .cancel(1),
        );
    }
}

impl<G: Game> Default for ReportScreen<G> {
    fn default() -> Self {
        ReportScreen::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Description,
    ReplyTo,
}

/// Whether `text` has the shape of an email address. The relay checks
/// that the domain takes mail.
fn looks_like_address(text: &str) -> bool {
    text.split_once('@').is_some_and(|(user, domain)| {
        !user.is_empty() && domain.contains('.') && !text.contains(char::is_whitespace)
    })
}

/// What a report that breaks a limit says, in the player's terms.
fn invalid_line(invalid: &Invalid) -> String {
    match invalid {
        Invalid::Empty("description") => DESCRIBE_FIRST.to_string(),
        Invalid::TooLong("description") => {
            "What happened is too long to send. Shorten it.".to_string()
        }
        Invalid::NotAnAddress => {
            "That does not look like an email address. Check it, or leave it blank.".to_string()
        }
        other => format!("The report cannot be sent: {other}."),
    }
}

impl<G: Game> Menu for ReportScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "report"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    /// After a crash the title says so, once, before the first row.
    fn enter(&mut self, ctx: &mut Ctx<G>) {
        if self.crashed {
            self.menu.title = format!(
                "Report a problem. {} closed unexpectedly the last time it ran. Say what you were doing when it happened",
                ctx.reports.name()
            );
        }
        crate::menu::menu_enter(self, ctx);
        self.menu.title = "Report a problem".to_string();
    }

    /// Back from the save question with the report saved: leave, saying
    /// where it is.
    fn resume(&mut self, ctx: &mut Ctx<G>) {
        let saved = self.draft.borrow_mut().saved.take();
        if let Some(path) = saved {
            ctx.pop_with(format!("The report was saved as {}.", path.display()));
            return;
        }
        self.refresh(ctx, true);
        self.announce(ctx);
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let mut items = vec![
            self.text_row("What happened", Field::Description),
            self.text_row("Your email address, for a reply", Field::ReplyTo),
        ];
        if ctx.reports.has_logs() {
            let draft = self.draft.clone();
            items.push(
                MenuItem::new(
                    Label::dynamic(move |_s: &Self, _ctx: &Ctx<G>| {
                        format!(
                            "Include the game log: {}",
                            if draft.borrow().include_logs {
                                "on"
                            } else {
                                "off"
                            }
                        )
                    }),
                    |s: &mut Self, ctx: &mut Ctx<G>| s.toggle_logs(ctx),
                )
                .help(
                    "The log records this session and the one before it, including everything \
                     the game said. It is the quickest way to find a problem. Enter, Left or \
                     Right changes it.",
                )
                .adjustable(|s: &mut Self, _ctx: &mut Ctx<G>, _delta| {
                    let mut d = s.draft.borrow_mut();
                    d.include_logs = !d.include_logs;
                }),
            );
        }
        items.push(
            MenuItem::new("Read the report", |s: &mut Self, ctx: &mut Ctx<G>| {
                s.read_report(ctx)
            })
            .help("Everything the report will say, a line at a time."),
        );
        items.push(
            MenuItem::new("Send", |s: &mut Self, ctx: &mut Ctx<G>| s.send(ctx))
                .help("Sends the report. The game's version and your operating system go with it."),
        );
        items
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        // The worker keeps going; the outcome is logged.
        menu_input(self, ctx, event);
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        if let Some(index) = self.focus_next.take() {
            self.menu.index = index.min(self.menu.items.len().saturating_sub(1));
            let current = self.current_text(ctx);
            ctx.say_queued(current);
        }
        let Some(outcome) = self.sending.as_mut().and_then(Sending::poll) else {
            return;
        };
        self.sending = None;
        match outcome {
            Ok(()) => {
                self.report = None;
                ctx.pop_with(SENT);
            }
            Err(SendError::ReplyAddressTakesNoMail) => {
                self.report = None;
                ctx.play_ui(UiSound::Error);
                ctx.say(BAD_ADDRESS);
                self.menu.index = 1;
                let current = self.current_text(ctx);
                ctx.say_queued(current);
            }
            Err(SendError::Invalid(invalid)) => {
                self.report = None;
                ctx.play_ui(UiSound::Error);
                ctx.say(invalid_line(&invalid));
            }
            Err(SendError::RateLimited) => {
                self.offer_to_save(ctx, "Too many reports have been sent today.".to_string());
            }
            Err(SendError::Rejected(reason) | SendError::Failed(reason)) => {
                self.offer_to_save(ctx, format!("The report could not be sent: {reason}"));
            }
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let mut out = vec![self.menu.title.clone()];
        if self.sending.is_some() {
            out.push("Sending the report.".to_string());
        }
        for (i, item) in self.menu.items.iter().enumerate() {
            let marker = if i == self.menu.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out
    }
}

/// The report read back a line at a time: the subject, the details, and
/// what is attached.
pub struct ReportPreviewScreen<G: Game> {
    menu: MenuCore<Self, G>,
    lines: Vec<String>,
}

impl<G: Game> ReportPreviewScreen<G> {
    pub fn new(report: &Report) -> Self {
        let mut lines = vec![report.subject()];
        lines.extend(
            report
                .body()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string),
        );
        for attachment in &report.attachments {
            lines.push(format!(
                "Attached: {}, {}.",
                attachment.name,
                kilobytes(attachment.text.len())
            ));
        }
        if report.attachments.is_empty() {
            lines.push("No log attached.".to_string());
        }
        ReportPreviewScreen {
            menu: MenuCore::new("The report")
                .with_intro_help("Up and Down read the report a line at a time, Escape goes back."),
            lines,
        }
    }
}

/// A size as spoken: "120 kilobytes".
fn kilobytes(bytes: usize) -> String {
    let kb = bytes.div_ceil(1000);
    if kb == 1 {
        "1 kilobyte".to_string()
    } else {
        format!("{kb} kilobytes")
    }
}

impl<G: Game> Menu for ReportPreviewScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "report_preview"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        self.lines
            .iter()
            .map(|line| MenuItem::inert(line.as_str()))
            .collect()
    }
}
