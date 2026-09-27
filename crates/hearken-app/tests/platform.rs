mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::TestGame;
use hearken_app::sdl_shell::translate_events;
use hearken_app::single_instance::{MutexApi, SingleInstanceGuard, ERROR_ALREADY_EXISTS};
use hearken_app::{already_running_line, InputEvent, Key, Mods};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};

#[test]
fn a_key_press_carries_the_text_typed_with_it_and_its_repeat_flag() {
    let raw = vec![
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(Keycode::A),
            scancode: None,
            keymod: Mod::LSHIFTMOD,
            repeat: true,
        },
        Event::TextInput {
            timestamp: 0,
            window_id: 0,
            text: "A".to_string(),
        },
    ];
    assert_eq!(
        translate_events(raw),
        [InputEvent::KeyDown {
            key: Key::A,
            mods: Mods::SHIFT,
            text: Some('A'),
            repeat: true
        }]
    );
}

#[test]
fn window_and_quit_events_translate() {
    let raw = vec![
        Event::Window {
            timestamp: 0,
            window_id: 0,
            win_event: WindowEvent::FocusLost,
        },
        Event::Window {
            timestamp: 0,
            window_id: 0,
            win_event: WindowEvent::FocusGained,
        },
        Event::Quit { timestamp: 0 },
    ];
    assert_eq!(
        translate_events(raw),
        [
            InputEvent::WindowFocusLost,
            InputEvent::WindowFocusGained,
            InputEvent::Quit
        ]
    );
}

struct FakeKernel {
    exists: bool,
    closed: Rc<Cell<u32>>,
}

impl MutexApi for FakeKernel {
    fn create_mutex(&self, _name: &str) -> (usize, u32) {
        (42, if self.exists { ERROR_ALREADY_EXISTS } else { 0 })
    }
    fn close_handle(&self, _handle: usize) {
        self.closed.set(self.closed.get() + 1);
    }
}

#[test]
fn a_second_copy_is_refused_and_closes_its_handle() {
    let closed = Rc::new(Cell::new(0));
    let mut guard = SingleInstanceGuard::with_api(
        "x",
        FakeKernel {
            exists: true,
            closed: closed.clone(),
        },
        true,
    );
    assert!(!guard.acquire());
    assert!(!guard.acquired());
    assert_eq!(closed.get(), 1);
}

#[test]
fn the_first_copy_holds_the_lock_until_released() {
    let kernel = FakeKernel {
        exists: false,
        closed: Rc::default(),
    };
    let mut guard = SingleInstanceGuard::with_api("x", kernel, true);
    assert!(guard.acquire());
    guard.release();
    assert!(!guard.acquired());
}

#[cfg(windows)]
#[test]
fn two_real_guards_on_one_name_exclude_each_other() {
    let name = hearken_app::single_instance::mutex_name("HearkenPlatformTest");
    let mut first = SingleInstanceGuard::new(&name);
    assert!(first.acquire());
    let mut second = SingleInstanceGuard::new(&name);
    assert!(!second.acquire());
    first.release();
    let mut third = SingleInstanceGuard::new(&name);
    assert!(third.acquire());
}

#[test]
fn the_already_running_line_names_the_game() {
    assert_eq!(
        already_running_line::<TestGame>(),
        "Test Game is already running."
    );
}

/// The only test here that sets up logging: the logger and the panic hook
/// are both process-wide.
#[test]
fn the_log_file_goes_to_the_data_folder_and_a_panic_is_logged_once() {
    // A player's RUST_LOG, set for some other tool, must not cost them the
    // transcript.
    std::env::set_var("RUST_LOG", "off");
    let dir = tempfile::tempdir().unwrap();
    let paths = hearken_app::Paths::at(dir.path());
    hearken_app::logging::init(&paths, "HEARKEN_TEST_PLATFORM");
    hearken_app::logging::init(&paths, "HEARKEN_TEST_PLATFORM");
    log::info!(target: hearken_app::TRANSCRIPT_TARGET, "a spoken line");
    let _ = std::panic::catch_unwind(|| panic!("the test panic"));
    log::logger().flush();
    let text = std::fs::read_to_string(paths.log_file()).unwrap();
    assert!(text.contains("a spoken line"), "log: {text}");
    assert_eq!(
        text.matches("Fatal error: ").count(),
        1,
        "one hook, however often logging starts: {text}"
    );
    assert!(text.contains("the test panic"), "log: {text}");
}

#[test]
fn starting_a_session_keeps_the_last_log_as_prev() {
    let dir = tempfile::tempdir().unwrap();
    let paths = hearken_app::Paths::at(dir.path());
    hearken_app::logging::rotate(&paths, "HEARKEN_TEST_ROTATE"); // nothing to keep yet
    std::fs::write(paths.log_file(), "first").unwrap();
    hearken_app::logging::rotate(&paths, "HEARKEN_TEST_ROTATE");
    std::fs::write(paths.log_file(), "second").unwrap();
    hearken_app::logging::rotate(&paths, "HEARKEN_TEST_ROTATE");
    assert!(!paths.log_file().exists());
    let prev = std::fs::read_to_string(dir.path().join("session.log.prev")).unwrap();
    assert_eq!(prev, "second", "the one before is replaced");
}
